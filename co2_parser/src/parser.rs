use chumsky::{
    input::{SliceInput, ValueInput},
    prelude::*,
};
use co2_ast::TypeResolver;
use co2_ast::*;

fn join_spans(start: Span, end: Span) -> Span {
    if start.context == end.context {
        Span {
            start: start.start,
            end: end.end,
            context: start.context,
        }
    } else {
        start
    }
}

fn single_token_span(slice: &[Spanned<Token>], fallback: Span) -> Span {
    slice.first().map(|(_, span)| *span).unwrap_or(fallback)
}

fn slice_span(slice: &[Spanned<Token>], fallback: Span) -> Span {
    slice.first()
        .zip(slice.last())
        .map(|(first, last)| join_spans(first.1, last.1))
        .unwrap_or(fallback)
}

fn look_ahead<'src, I>(
    token: Token,
) -> impl Parser<'src, I, (), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    custom(move |inp| {
        let check_point = inp.save();
        let before = inp.cursor();
        match inp.next() {
            Some(t) if t == token => {
                inp.rewind(check_point);
                Ok(())
            }
            Some(t) => Err(Rich::custom(
                inp.span_since(&before),
                format!("expected {token}, found {t}"),
            )),
            None => Err(Rich::custom(
                inp.span_since(&before),
                format!("unexpected eof, expected {token}"),
            )),
        }
    })
}

fn lazy_compound_statement<'src, I>()
-> impl Parser<'src, I, Spanned<LazyCompoundStatement>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|block| {
        let content = choice((
            // Skip any token that isn't a brace
            any()
                .filter(|t| !matches!(t, Token::LBrace | Token::RBrace))
                .ignored(),
            // Recursively skip balanced blocks
            block.ignored(),
        ))
        .repeated()
        .ignored();

        content.delimited_by(just(Token::LBrace), just(Token::RBrace))
    })
    .map_with(|_, e| {
        let slice = e.slice();
        let span = slice_span(slice, e.span());
        (
            LazyCompoundStatement {
                tokens: (<[_]>::to_vec(slice), span),
            },
            span,
        )
    })
}

fn repeated_statement_with_modified_resolver<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Vec<Spanned<StatementOrDeclaration<R>>>, extra::Err<Rich<'src, Token, Span>>>
+ Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    fn is_finished<'src, I>(
        inp: &mut chumsky::input::InputRef<
            'src,
            '_,
            I,
            extra::Full<Rich<'src, Token, Span>, (), ()>,
        >,
    ) -> bool
    where
        I: ValueInput<'src, Token = Token, Span = Span>
            + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
    {
        let check_point = inp.save();
        let is_finished = inp.next() == Some(Token::RBrace);
        inp.rewind(check_point);
        is_finished
    }
    custom(move |inp| {
        if is_finished(inp) {
            return Ok(vec![]);
        }
        let mut current_resolver = resolver.start_new_scope();
        let mut result = vec![];
        loop {
            if is_finished(inp) {
                return Ok(result);
            }
            let (item, nr) = inp.parse(statement_or_declaration(
                current_resolver.clone(),
                statement(current_resolver),
            ))?;
            current_resolver = nr;
            result.push(item);
        }
    })
}

pub(crate) fn compound_statement<'src, I, R: TypeResolver>(
    resolver: R,
    _stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<CompoundStatement<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    repeated_statement_with_modified_resolver(resolver)
        .map(|statements| CompoundStatement { statements })
        .map_with(|r, e| (r, e.span()))
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
}

fn statement_or_declaration<'src, I, R: TypeResolver>(
    resolver: R,
    stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, (Spanned<StatementOrDeclaration<R>>, R), extra::Err<Rich<'src, Token, Span>>>
+ Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    declaration(resolver.clone(), stmt_rec.clone())
        .map(|(v, r)| (StatementOrDeclaration::Declaration(v), r))
        .or(stmt_rec.map(move |v| (StatementOrDeclaration::Statement(v), resolver.clone())))
        .map_with(|(v, r), e| ((v, e.span()), r))
}

pub(crate) fn statement<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|stmt_rec| {
        let assign_expression_rec = assignment_expression(resolver.clone(), stmt_rec.clone());
        let expression_rec = expression(assign_expression_rec);
        let jump_statement = just(Token::Return)
            .ignore_then(expression_rec.clone().or_not())
            .map(|exp| Statement::Return(exp))
            .then_ignore(just(Token::Semicolon));
        let goto_statement = just(Token::Goto)
            .ignore_then(identifier())
            .then_ignore(just(Token::Semicolon))
            .map(Statement::Goto);
        let break_statement = just(Token::Break)
            .then_ignore(just(Token::Semicolon))
            .to(Statement::Break);
        let continue_statement = just(Token::Continue)
            .then_ignore(just(Token::Semicolon))
            .to(Statement::Continue);
        let switch_statement = just(Token::Switch)
            .ignore_then(
                expression_rec
                    .clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then(stmt_rec.clone())
            .map(|(expr, body)| Statement::Switch {
                expr,
                body: Box::new(body),
            });
        let case_statement = just(Token::Case)
            .ignore_then(expression_rec.clone())
            .then_ignore(just(Token::Colon))
            .then(stmt_rec.clone())
            .map(|(expr, statement)| Statement::Case {
                expr,
                statement: Box::new(statement),
            });
        let default_statement = just(Token::Default)
            .ignore_then(just(Token::Colon).ignore_then(stmt_rec.clone()))
            .map(|statement| Statement::Default {
                statement: Box::new(statement),
            });

        let expression_statement = expression_rec
            .clone()
            .map(Statement::Expression)
            .then_ignore(just(Token::Semicolon));
        let empty_statement = just(Token::Semicolon).to(Statement::Empty);

        let compound =
            compound_statement(resolver.clone(), stmt_rec.clone()).map(Statement::Compound);

        let if_statement = just(Token::If)
            .ignore_then(
                expression_rec
                    .clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then(stmt_rec.clone())
            .then(just(Token::Else).ignore_then(stmt_rec.clone()).or_not())
            .map(|((cond, then_branch), else_branch)| Statement::If {
                cond,
                then_branch: Box::new(then_branch),
                else_branch: else_branch.map(Box::new),
            });

        let while_statement = just(Token::While)
            .ignore_then(
                expression_rec
                    .clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then(stmt_rec.clone())
            .map(|(cond, body)| Statement::While {
                cond,
                body: Box::new(body),
            });
        let do_while_statement = just(Token::Do)
            .ignore_then(stmt_rec.clone())
            .then_ignore(just(Token::While))
            .then(
                expression_rec
                    .clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then_ignore(just(Token::Semicolon))
            .map(|(body, cond)| Statement::DoWhile {
                body: Box::new(body),
                cond,
            });
        let labeled_statement = identifier()
            .then_ignore(just(Token::Colon))
            .then(stmt_rec.clone())
            .map(|(name, statement)| Statement::Label {
                name,
                statement: Box::new(statement),
            });
        let labeled_or_expression_statement = custom({
            let expression_statement = expression_statement.clone();
            let labeled_statement = labeled_statement.clone();
            move |inp| {
                let checkpoint = inp.save();
                let is_label = inp
                    .parse(identifier().ignored().then_ignore(look_ahead(Token::Colon)))
                    .is_ok();
                inp.rewind(checkpoint);
                if is_label {
                    inp.parse(labeled_statement.clone())
                } else {
                    inp.parse(expression_statement.clone())
                }
            }
        });

        let for_statement = just(Token::For).ignore_then(custom({
            let resolver = resolver.clone();
            let stmt_rec = stmt_rec.clone();
            move |inp| {
                inp.parse(just(Token::LParen))?;

                let init_resolver = resolver.clone();
                let (init, loop_resolver) = {
                    let checkpoint = inp.save();
                    if let Ok((decl, next_resolver)) =
                        inp.parse(declaration(init_resolver.clone(), stmt_rec.clone()))
                    {
                        (Some(ForInit::Declaration(decl)), next_resolver)
                    } else {
                        inp.rewind(checkpoint);
                        let expr = inp.parse(
                            expression_rec
                                .clone()
                                .or_not()
                                .then_ignore(just(Token::Semicolon)),
                        )?;
                        (
                            expr.map(ForInit::Expression),
                            init_resolver,
                        )
                    }
                };

                let loop_expr = expression(
                    assignment_expression(loop_resolver.clone(), stmt_rec.clone()),
                );
                let cond = inp.parse(loop_expr.clone().or_not().then_ignore(just(Token::Semicolon)))?;
                let post = inp.parse(loop_expr.or_not())?;
                inp.parse(just(Token::RParen))?;
                let body = inp.parse(statement(loop_resolver))?;

                Ok(Statement::For {
                    init,
                    cond,
                    post,
                    body: Box::new(body),
                })
            }
        }));

        choice((
            if_statement,
            while_statement,
            do_while_statement,
            for_statement,
            switch_statement,
            case_statement,
            default_statement,
            labeled_or_expression_statement,
            goto_statement,
            break_statement,
            continue_statement,
            jump_statement,
            empty_statement,
            compound,
        ))
        .map_with(|r, e| (r, e.span()))
    })
}

pub(crate) fn expression<'src, I, R: TypeResolver>(
    assignment: impl Parser<'src, I, Spanned<Expression<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<Expression<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let comma = assignment
        .separated_by(just(Token::Comma))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|x| {
            let mut x = x.into_iter();
            let mut r = x.next().unwrap();
            for x in x {
                let span = r.1;
                r = (
                    Expression::BinOp(Box::new(r), BinOp::Comma, Box::new(x)),
                    span,
                );
            }
            r
        });

    comma
}

pub(crate) fn assignment_expression<'src, I, R: TypeResolver>(
    resolver: R,
    stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<Expression<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        #[derive(Clone)]
        enum PostfixPart<R: TypeResolver> {
            Subscript(Spanned<Expression<R>>),
            Call(Vec<Spanned<Expression<R>>>),
            Dot(Spanned<String>),
            Arrow(Spanned<String>),
            PostInc,
            PostDec,
        }

        let compound_initializer = recursive(|init_rec| {
            let designator = choice((
                rec.clone()
                    .delimited_by(just(Token::LBracket), just(Token::RBracket))
                    .map(Designator::Subscript),
                just(Token::Dot)
                    .ignore_then(identifier())
                    .map(Designator::Field),
            ))
            .map_with(|r, e| (r, e.span()));

            let initializer_item = designator
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>()
                .then_ignore(just(Token::Assign))
                .or_not()
                .then(init_rec.clone())
                .map(|(designators, initializer)| InitializerItem {
                    designators,
                    initializer,
                })
                .map_with(|r, e| (r, e.span()));

            let list = initializer_item
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace))
                .map(Initializer::List)
                .map_with(|r, e| (r, e.span()));

            let expr = rec
                .clone()
                .map(Initializer::Expr)
                .map_with(|r, e| (r, e.span()));

            choice((list, expr))
        });

        let compound_literal = type_name(resolver.clone(), rec.clone())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .then(
                compound_initializer
                    .clone()
                    .filter(|(init, _)| matches!(init, Initializer::List(_))),
            )
            .map(|(type_name, initializer)| Expression::CompoundLiteral {
                type_name: Box::new(type_name),
                initializer: Box::new(initializer),
            });

        let va_exp = choice((
            just(Token::VaStart).ignore_then(
                rec.clone()
                    .then_ignore(just(Token::Comma))
                    .then(identifier())
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .map(|(args, last_param)| {
                        let args = Box::new(args);
                        Expression::VaStart { args, last_param }
                    }),
            ),
            just(Token::VaArg).ignore_then(
                rec.clone()
                    .then_ignore(just(Token::Comma))
                    .then(type_name(resolver.clone(), rec.clone()))
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .map(|(args, type_name)| {
                        let args = Box::new(args);
                        Expression::VaArg { args, type_name }
                    }),
            ),
            just(Token::VaEnd).ignore_then(
                rec.clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .map(|args| {
                        let args = Box::new(args);
                        Expression::VaEnd { args }
                    }),
            ),
        ));

        let generic_selection = just(Token::Generic).ignore_then(
            rec.clone()
                .then_ignore(just(Token::Comma))
                .then(
                    choice((
                        just(Token::Default)
                            .ignore_then(just(Token::Colon))
                            .ignore_then(rec.clone())
                            .map(|expr| GenericAssociation::Default { expr }),
                        type_name(resolver.clone(), rec.clone())
                            .then_ignore(just(Token::Colon))
                            .then(rec.clone())
                            .map(|(type_name, expr)| GenericAssociation::Type { type_name, expr }),
                    ))
                    .map_with(|assoc, e| (assoc, e.span()))
                    .separated_by(just(Token::Comma))
                    .at_least(1)
                    .collect::<Vec<_>>(),
                )
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .map(|(controlling, associations)| Expression::GenericSelection {
                    controlling: Box::new(controlling),
                    associations,
                }),
        );

        let primary_expression = choice((
            compound_literal,
            va_exp,
            generic_selection,
            just(Token::BuiltinInf)
                .then_ignore(just(Token::LParen))
                .then_ignore(just(Token::RParen))
                .to(Expression::Constant(Constant::Float(f64::INFINITY))),
            just(Token::BuiltinNan)
                .then_ignore(just(Token::LParen))
                .then_ignore(
                    select! {
                        Token::StringLit(s) => s,
                    }
                    .repeated()
                    .at_least(1)
                    .collect::<Vec<_>>()
                    .or_not(),
                )
                .then_ignore(just(Token::RParen))
                .to(Expression::Constant(Constant::Float(f64::NAN))),
            just(Token::LParen)
                .then_ignore(look_ahead(Token::LBrace))
                .ignore_then(stmt_rec.clone())
                .then_ignore(just(Token::RParen))
                .map(|(body, _)| Expression::GnuStatementExpr {
                    body: Box::new({
                        let Statement::Compound(body) = body else {
                            unreachable!();
                        };
                        body
                    }),
                }),
            rust_path().try_map({
                let resolver = resolver.clone();
                move |path, _| match resolver.classify_path(&path.0) {
                    Some((TypeQueryResult::Unsure | TypeQueryResult::Expr, resolved)) => {
                        Ok(Expression::Identifier((resolved, path.1)))
                    }
                    Some((TypeQueryResult::Type, _)) => {
                        Err(Rich::custom(path.1, "expected expression, found type name"))
                    }
                    None => Err(Rich::custom(path.1, "Unresolved name")),
                }
            }),
            select! {
                Token::StringLit(s) => s,
            }
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|parts| Expression::Constant(Constant::String(parts.concat()))),
            select! {
                Token::Integer(i, suffix) => Expression::Constant(Constant::Int(parse_integer_constant(&i), suffix)),
                Token::FloatLit(i, _) => Expression::Constant(Constant::Float(i.parse().unwrap())),
                Token::CharLit(s) => {
                    let ch = s.chars().next().expect("empty char literal");
                    Expression::Constant(Constant::Char(ch))
                },
            },
            expression(rec.clone())
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .map(|x: Spanned<Expression<R>>| x.0),
        ))
        .map_with(|r, e| (r, e.span()));

        let postfix_expression = primary_expression
            .then(
                choice((
                    rec.clone()
                        .delimited_by(just(Token::LBracket), just(Token::RBracket))
                        .map(PostfixPart::<R>::Subscript),
                    rec.clone()
                        .separated_by(just(Token::Comma))
                        .collect()
                        .map(PostfixPart::<R>::Call)
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                    just(Token::Dot)
                        .ignore_then(identifier())
                        .map(PostfixPart::<R>::Dot),
                    just(Token::Arrow)
                        .ignore_then(identifier())
                        .map(PostfixPart::<R>::Arrow),
                    just(Token::Inc).to(PostfixPart::<R>::PostInc),
                    just(Token::Dec).to(PostfixPart::<R>::PostDec),
                ))
                .repeated()
                .collect::<Vec<PostfixPart<R>>>(),
            )
            .map(|(mut main, posts)| {
                for post in posts {
                    let span = main.1;
                    let post_expr = match post {
                        PostfixPart::Subscript(sub) => {
                            Expression::Subscript(Box::new(main), Box::new(sub))
                        }
                        PostfixPart::Call(params) => Expression::Call {
                            func: Box::new(main),
                            params,
                        },
                        PostfixPart::Dot(ident) => Expression::Field(Box::new(main), ident),
                        PostfixPart::Arrow(ident) => Expression::Arrow(Box::new(main), ident),
                        PostfixPart::PostInc => Expression::Update {
                            expr: Box::new(main),
                            op: UpdateOp::Inc,
                            is_postfix: true,
                        },
                        PostfixPart::PostDec => Expression::Update {
                            expr: Box::new(main),
                            op: UpdateOp::Dec,
                            is_postfix: true,
                        },
                    };
                    main = (post_expr, span);
                }
                main
            });

        let unary_op = choice((
            just(Token::Bang).to(UnaryOp::Not),
            just(Token::Tilde).to(UnaryOp::Com),
            just(Token::Amp).to(UnaryOp::AddrOf),
            just(Token::Star).to(UnaryOp::Deref),
            just(Token::Plus).to(UnaryOp::Plus),
            just(Token::Minus).to(UnaryOp::Minus),
        ));

        let cast_expression = recursive(|cast_expr| {
            let unary_expression = recursive(|unary| {
                let sizeof_type_expression = just(Token::Sizeof)
                    .ignore_then(
                        type_name(resolver.clone(), rec.clone())
                            .delimited_by(just(Token::LParen), just(Token::RParen)),
                    )
                    .map(|ty| Expression::SizeofType(Box::new(ty)))
                    .map_with(|r, e| (r, e.span()));

                let offsetof_expression = just(Token::Offsetof)
                    .ignore_then(just(Token::LParen))
                    .ignore_then(type_name(resolver.clone(), rec.clone()))
                    .then_ignore(just(Token::Comma))
                    .then(select! { Token::Ident(s) => s })
                    .then_ignore(just(Token::RParen))
                    .map(|(ty, field)| Expression::Offsetof {
                        ty: Box::new(ty),
                        field,
                    })
                    .map_with(|r, e| (r, e.span()));

                let prefix_inc_expression = just(Token::Inc)
                    .ignore_then(unary.clone())
                    .map(|expr| Expression::Update {
                        expr: Box::new(expr),
                        op: UpdateOp::Inc,
                        is_postfix: false,
                    })
                    .map_with(|r, e| (r, e.span()));

                let prefix_dec_expression = just(Token::Dec)
                    .ignore_then(unary.clone())
                    .map(|expr| Expression::Update {
                        expr: Box::new(expr),
                        op: UpdateOp::Dec,
                        is_postfix: false,
                    })
                    .map_with(|r, e| (r, e.span()));

                let unary_operator_expression = unary_op
                    .then(cast_expr.clone())
                    .map(|(op, expr)| Expression::UnaryOp(op, Box::new(expr)))
                    .map_with(|r, e| (r, e.span()));

                let sizeof_unary_expression = just(Token::Sizeof)
                    .ignore_then(unary)
                    .map(|expr| Expression::Sizeof(Box::new(expr)))
                    .map_with(|r, e| (r, e.span()));

                choice((
                    sizeof_type_expression,
                    offsetof_expression,
                    prefix_inc_expression,
                    prefix_dec_expression,
                    unary_operator_expression,
                    sizeof_unary_expression,
                    postfix_expression.clone(),
                ))
            });

            let cast_type_expression = type_name(resolver.clone(), rec.clone())
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .then(cast_expr.clone())
                .map(|(type_name, expr)| Expression::Cast {
                    type_name: Box::new(type_name),
                    expr: Box::new(expr),
                })
                .map_with(|r, e| (r, e.span()));

            choice((cast_type_expression, unary_expression))
        });

        let mul = just(Token::Star).to(BinOp::Mul);
        let div = just(Token::Slash).to(BinOp::Div);
        let rem = just(Token::Percent).to(BinOp::Rem);
        let add = just(Token::Plus).to(BinOp::Add);
        let sub = just(Token::Minus).to(BinOp::Sub);
        let shl = just(Token::Shl).to(BinOp::Shl);
        let shr = just(Token::Shr).to(BinOp::Shr);
        let lt = just(Token::Lt).to(BinOp::Lt);
        let le = just(Token::Le).to(BinOp::Le);
        let gt = just(Token::Gt).to(BinOp::Gt);
        let ge = just(Token::Ge).to(BinOp::Ge);
        let eq = just(Token::EqEq).to(BinOp::Eq);
        let ne = just(Token::Ne).to(BinOp::Ne);
        let bit_and = just(Token::Amp).to(BinOp::BitAnd);
        let bit_xor = just(Token::Caret).to(BinOp::BitXor);
        let bit_or = just(Token::Pipe).to(BinOp::BitOr);
        let logical_and = just(Token::And).to(BinOp::And);
        let logical_or = just(Token::Or).to(BinOp::Or);

        let multiplicative = cast_expression
            .clone()
            .then(
                choice((mul, div, rem))
                    .then(cast_expression)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let additive = multiplicative
            .clone()
            .then(
                choice((add, sub))
                    .then(multiplicative)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let shift = additive
            .clone()
            .then(
                choice((shl, shr))
                    .then(additive)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let relational = shift
            .clone()
            .then(
                choice((lt, le, gt, ge))
                    .then(shift)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let equality = relational
            .clone()
            .then(
                choice((eq, ne))
                    .then(relational)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let bit_and_expr = equality
            .clone()
            .then(bit_and.then(equality).repeated().collect::<Vec<_>>())
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let bit_xor_expr = bit_and_expr
            .clone()
            .then(bit_xor.then(bit_and_expr).repeated().collect::<Vec<_>>())
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let bit_or_expr = bit_xor_expr
            .clone()
            .then(bit_or.then(bit_xor_expr).repeated().collect::<Vec<_>>())
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let logical_and_expr = bit_or_expr
            .clone()
            .then(logical_and.then(bit_or_expr).repeated().collect::<Vec<_>>())
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        let logical_or_expr = logical_and_expr
            .clone()
            .then(
                logical_or
                    .then(logical_and_expr)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            })
            .boxed();

        // C grammar: logical-OR-expression ? expression : conditional-expression
        // The "then" branch is a full expression (comma operator allowed);
        // the "else" branch is conditional-expression (no top-level comma).
        let conditional_expr = logical_or_expr
            .clone()
            .then(
                just(Token::Question)
                    .ignore_then(expression(rec.clone()))
                    .then_ignore(just(Token::Colon))
                    .then(rec.clone())
                    .map(|(then_expr, else_expr)| (then_expr, else_expr))
                    .or_not(),
            )
            .map(|(cond, then_else)| {
                if let Some((then_expr, else_expr)) = then_else {
                    let span = cond.1;
                    (
                        Expression::Conditional {
                            cond: Box::new(cond),
                            then_expr: Box::new(then_expr),
                            else_expr: Box::new(else_expr),
                        },
                        span,
                    )
                } else {
                    cond
                }
            })
            .boxed();

        let assignment = conditional_expr
            .clone()
            .then(
                choice((
                    just(Token::Assign).to(None),
                    just(Token::PlusAssign).to(Some(BinOp::Add)),
                    just(Token::MinusAssign).to(Some(BinOp::Sub)),
                    just(Token::StarAssign).to(Some(BinOp::Mul)),
                    just(Token::SlashAssign).to(Some(BinOp::Div)),
                    just(Token::PercentAssign).to(Some(BinOp::Rem)),
                    just(Token::PipeAssign).to(Some(BinOp::BitOr)),
                    just(Token::CaretAssign).to(Some(BinOp::BitXor)),
                    just(Token::AmpAssign).to(Some(BinOp::BitAnd)),
                    just(Token::ShlAssign).to(Some(BinOp::Shl)),
                    just(Token::ShrAssign).to(Some(BinOp::Shr)),
                ))
                .then(rec.clone())
                .or_not(),
            )
            .map(|(lhs, assign)| {
                if let Some((op, rhs)) = assign {
                    let span = lhs.1;
                    match op {
                        Some(op) => (
                            Expression::AssignWithOp {
                                lhs: Box::new(lhs),
                                op,
                                rhs: Box::new(rhs),
                            },
                            span,
                        ),
                        None => (
                            Expression::BinOp(Box::new(lhs), BinOp::Assign, Box::new(rhs)),
                            span,
                        ),
                    }
                } else {
                    lhs
                }
            })
            .map_with(|r, e| (r.0, e.span()));

        assignment
    })
}

fn parse_integer_constant(text: &str) -> i128 {
    parse_unsigned_integer_constant(text)
        .map(|v| v as i128)
        .unwrap_or_else(|e| panic!("invalid integer literal `{text}`: {e}"))
}

fn lazy_subscription<'src, I>()
-> impl Parser<'src, I, Spanned<LazySubscription>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|block| {
        let content = choice((
            // Skip any token that isn't a brace
            any()
                .filter(|t| !matches!(t, Token::LBracket | Token::RBracket))
                .ignored(),
            // Recursively skip balanced blocks
            block.ignored(),
        ))
        .repeated()
        .ignored();

        content.delimited_by(just(Token::LBracket), just(Token::RBracket))
    })
    .map_with(|_, e| {
        let slice = e.slice();
        let span = slice_span(slice, e.span());
        (
            LazySubscription {
                tokens: <[_]>::to_vec(slice),
            },
            span,
        )
    })
}

fn rust_path_with_generic_args<'src, I>(
    generic_ty: impl Parser<
        'src,
        I,
        Spanned<RustTy<StatelessResolver>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<RustPath>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let generics = generic_ty
        .separated_by(just(Token::Comma))
        .collect()
        .map(RustPathSegment::Generics)
        .delimited_by(just(Token::Lt), just(Token::Gt))
        .map_with(|r, e| (r, e.span()));

    choice((
        identifier()
            .then(generics.clone().or_not())
            .map(|(ident, generics)| {
                let mut segments = vec![(RustPathSegment::Ident(ident.0), ident.1)];
                if let Some(generics) = generics {
                    segments.push(generics);
                }
                segments
            }),
        generics.map(|generics| vec![generics]),
    ))
    .separated_by(just(Token::ColonColon))
    .at_least(1)
    .collect::<Vec<Vec<Spanned<RustPathSegment>>>>()
    .map(|parts| {
        let segments = parts
            .into_iter()
            .flatten()
            .collect::<Vec<Spanned<RustPathSegment>>>();
        let span = segments
            .first()
            .zip(segments.last())
            .map(|(first, last)| join_spans(first.1, last.1))
            .unwrap_or(Span {
                start: 0,
                end: 0,
                context: FileId::INVALID,
            });
        (RustPath { segments }, span)
    })
}

fn rust_path<'src, I>()
-> impl Parser<'src, I, Spanned<RustPath>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_path_with_generic_args(rust_generic_arg_ty())
}

fn rust_generic_arg_ty<'src, I>()
-> impl Parser<'src, I, Spanned<RustTy<StatelessResolver>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let path = rust_path_with_generic_args(rec.clone())
            .map(|path| (RustTy::Path((path.0, path.1)), path.1));

        let ptr = just(Token::Star)
            .ignore_then(choice((
                just(Token::Const).to(false),
                mut_token().to(true),
            )))
            .then(rec.clone())
            .map(|(mutable, inner)| RustTy::Ptr {
                mutable,
                inner: Box::new(inner),
            })
            .map_with(|r, e| (r, e.span()));

        let reference = just(Token::Amp)
            .ignore_then(mut_token().to(true).or_not().map(|m| m.is_some()))
            .then(rec.clone())
            .map(|(mutable, inner)| RustTy::Ref {
                mutable,
                inner: Box::new(inner),
            })
            .map_with(|r, e| (r, e.span()));

        let never = just(Token::Bang)
            .to(RustTy::Never)
            .map_with(|r, e| (r, e.span()));

        let tuple = rec
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(RustTy::Tuple)
            .map_with(|r, e| (r, e.span()));

        let slice_or_array = rec
            .clone()
            .then(
                just(Token::Semicolon)
                    .ignore_then(
                        any()
                            .filter(|t| !matches!(t, Token::RBracket))
                            .map_with(|t, e| (t, e.span()))
                            .repeated()
                            .collect()
                            .map(|tokens| LazyRustConstExpr { tokens }),
                    )
                    .map_with(|r, e| (r, e.span()))
                    .or_not(),
            )
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map(|(inner, len)| {
                if let Some(len) = len {
                    RustTy::Array {
                        inner: Box::new(inner),
                        len,
                    }
                } else {
                    RustTy::Slice(Box::new(inner))
                }
            })
            .map_with(|r, e| (r, e.span()));

        choice((path, ptr, reference, never, tuple, slice_or_array))
    })
}

fn left_recursion<'src, I, B: 'src, E: 'src>(
    base: impl Parser<'src, I, B, extra::Err<Rich<'src, Token, Span>>> + Clone + 'src,
    left_elem: impl Parser<'src, I, E, extra::Err<Rich<'src, Token, Span>>> + Clone + 'src,
) -> impl Parser<'src, I, (Vec<E>, B), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(
        |rec: Recursive<dyn Parser<'src, I, (Vec<E>, B), extra::Err<Rich<'src, Token, Span>>>>| {
            let base = left_elem.clone().then(base).map(|(e, b)| (vec![e], b));
            let rec_case = left_elem.then(rec).map(|(first_elem, mut result)| {
                result.0.insert(0, first_elem);
                result
            });
            choice((base, rec_case))
        },
    )
}

fn struct_or_union_fields<'src, I, R: TypeResolver>(
    type_specifier_rec: impl Parser<
        'src,
        I,
        Spanned<TypeSpecifier<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Vec<Spanned<StructOrUnionField<R>>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let single = left_recursion(
        struct_declarator(declarator_rec)
            .separated_by(just(Token::Comma))
            .collect()
            .then_ignore(just(Token::Semicolon)),
        choice((
            type_specifier_rec.map(SpecifierQualifier::TypeSpecifier),
            type_qualifier().map(SpecifierQualifier::TypeQualifier),
        ))
        .map_with(|r, e| (r, e.span())),
    )
    .map(|(specs, declarators)| StructOrUnionField {
        specifiers: specs,
        declarators,
    })
    .map_with(|r, e| (r, e.span()));
    single
        .repeated()
        .collect()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
}

fn type_specifier<'src, I, R: TypeResolver>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    resolver: R,
) -> impl Parser<'src, I, Spanned<TypeSpecifier<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let struct_or_union_specifier = group((
            just(Token::Struct)
                .to(StructOrUnionKind::Struct)
                .or(just(Token::Union).to(StructOrUnionKind::Union)),
            choice((
                identifier()
                    .then(struct_or_union_fields(rec.clone(), declarator_rec.clone()))
                    .map(|(ident, fields)| StructOrUnionSpecifier::Defined { ident, fields }),
                struct_or_union_fields(rec, declarator_rec)
                    .map(|fields| StructOrUnionSpecifier::Anonymous { fields }),
                identifier().map(|ident| StructOrUnionSpecifier::Declared { ident }),
            ))
            .map_with(|r, e| (r, e.span())),
        ))
        .map({
            let resolver = resolver.clone();
            move |(kind, (specifier, span))| TypeSpecifier::StructOrUnion {
                kind,
                specifier: (
                    resolver.register_struct_or_union_specifier(kind, (specifier, span)),
                    span,
                ),
            }
        });

        let enumerator = identifier()
            .then(
                just(Token::Assign)
                    .ignore_then(assign_expression_rec)
                    .or_not(),
            )
            .map(|(ident, value)| Enumerator { ident, value })
            .map_with(|r, e| (r, e.span()))
            .map({
                let resolver = resolver.clone();
                move |x| {
                    let span = x.1;
                    (resolver.register_enumerator(x), span)
                }
            });
        let enum_body = enumerator
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace));
        let enum_specifier = just(Token::Enum)
            .ignore_then(
                choice((
                    identifier()
                        .then(enum_body.clone())
                        .map(|(ident, enumerators)| EnumSpecifier::Defined { ident, enumerators }),
                    enum_body
                        .clone()
                        .map(|enumerators| EnumSpecifier::Anonymous { enumerators }),
                    identifier().map(|ident| EnumSpecifier::Declared { ident }),
                ))
                .map_with(|r, e| (r, e.span())),
            )
            .map({
                let resolver = resolver.clone();
                move |(x, span)| {
                    TypeSpecifier::Enum((resolver.register_enum_specifier((x, span)), span))
                }
            })
            .boxed();

        choice([
            just(Token::Int).to(TypeSpecifier::Int),
            just(Token::Bool).to(TypeSpecifier::Bool),
            just(Token::Void).to(TypeSpecifier::Void),
            just(Token::Char).to(TypeSpecifier::Char),
            just(Token::Short).to(TypeSpecifier::Short),
            just(Token::Long).to(TypeSpecifier::Long),
            just(Token::Float).to(TypeSpecifier::Float),
            just(Token::Double).to(TypeSpecifier::Double),
            just(Token::Signed).to(TypeSpecifier::Signed),
            just(Token::Unsigned).to(TypeSpecifier::Unsigned),
        ])
        .or(struct_or_union_specifier)
        .or(enum_specifier)
        .or(rust_path()
            .try_map({
                let resolver = resolver.clone();
                move |path, _| match resolver.classify_path(&path.0) {
                    Some((TypeQueryResult::Unsure | TypeQueryResult::Type, resolved)) => {
                        Ok(TypeSpecifier::TypedefName((resolved, path.1)))
                    }
                    Some((TypeQueryResult::Expr, _)) => {
                        Err(Rich::custom(path.1, "expected type name, found expression"))
                    }
                    None => Err(Rich::custom(path.1, "Unresolved name")),
                }
            }))
        .map_with(|r, e| (r, e.span()))
        .labelled("Type specifier")
    })
}

fn storage_class_specifier<'src, I>()
-> impl Parser<'src, I, Spanned<StorageClassSpecifier>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice([
        just(Token::Typedef).to(StorageClassSpecifier::Typedef),
        just(Token::Extern).to(StorageClassSpecifier::Extern),
        just(Token::Static).to(StorageClassSpecifier::Static),
        just(Token::Atomic).to(StorageClassSpecifier::Atomic),
        // just(Token::ThreadLocal).to(StorageClassSpecifier::ThreadLocal),
        just(Token::Auto).to(StorageClassSpecifier::Auto),
        just(Token::Register).to(StorageClassSpecifier::Register),
    ])
    .labelled("Storage specifier")
    .map_with(|r, e| (r, e.span()))
}

fn type_qualifier<'src, I>()
-> impl Parser<'src, I, Spanned<TypeQualifier>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice([
        just(Token::Const).to(TypeQualifier::Const),
        just(Token::Restrict).to(TypeQualifier::Restrict),
        just(Token::Volatile).to(TypeQualifier::Volatile),
    ])
    .labelled("Type qualifier")
    .map_with(|r, e| (r, e.span()))
}

fn function_specifier<'src, I>()
-> impl Parser<'src, I, Spanned<FunctionSpecifier>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice([just(Token::Inline).to(FunctionSpecifier::Inline)])
        .labelled("Function specifier")
        .map_with(|r, e| (r, e.span()))
}

fn specifier_qualifier<'src, I, R: TypeResolver>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    resolver: R,
) -> impl Parser<'src, I, Spanned<SpecifierQualifier<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice((
        type_specifier(declarator_rec, assign_expression_rec, resolver)
            .map(SpecifierQualifier::TypeSpecifier),
        type_qualifier().map(SpecifierQualifier::TypeQualifier),
    ))
    .map_with(|r, e| (r, e.span()))
}

fn type_name<'src, I, R: TypeResolver>(
    resolver: R,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
) -> impl Parser<'src, I, TypeName<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let declarator = declarator(resolver.clone(), assign_expression_rec.clone());
    let sq = specifier_qualifier(declarator.clone(), assign_expression_rec, resolver)
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>();
    sq.then(declarator.or_not())
        .map(|(specifier_qualifier_list, abstract_declarator)| TypeName {
            specifier_qualifier_list,
            abstract_declarator: abstract_declarator.and_then(|decl| {
                if matches!(decl.0, Declarator::Abstract) {
                    None
                } else {
                    Some(decl)
                }
            }),
        })
}

fn declaration_specifier<'src, I, R: TypeResolver>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    resolver: R,
) -> impl Parser<'src, I, Spanned<DeclarationSpecifier<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice((
        type_specifier(declarator_rec, assign_expression_rec, resolver)
            .map(DeclarationSpecifier::TypeSpecifier),
        type_qualifier().map(DeclarationSpecifier::TypeQualifier),
        storage_class_specifier().map(DeclarationSpecifier::StorageSpecifier),
        function_specifier().map(DeclarationSpecifier::FunctionSpecifier),
    ))
    .map_with(|r, e| (r, e.span()))
}

fn fn_token<'src, I>() -> impl Parser<'src, I, (), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! { Token::Ident(s) if s == "fn" => () }.labelled("fn")
}

fn mut_token<'src, I>() -> impl Parser<'src, I, (), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! { Token::Ident(s) if s == "mut" => () }.labelled("mut")
}

fn identifier<'src, I>()
-> impl Parser<'src, I, Spanned<String>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! {
        Token::Ident(s) => s,
    }
    .labelled("Identifier")
    .map_with(|r, e| (r, single_token_span(e.slice(), e.span())))
}

fn number<'src, I>()
-> impl Parser<'src, I, Spanned<String>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! {
        Token::Integer(s, _) | Token::FloatLit(s, _) => s,
    }
    .labelled("Number")
    .map_with(|r, e| (r, single_token_span(e.slice(), e.span())))
}

fn parameter_type_list<'src, I, R: TypeResolver>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    resolver: R,
) -> impl Parser<'src, I, ParameterList<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let single = left_recursion(
        declarator_rec
            .clone()
            .then_ignore(look_ahead(Token::RParen).or(look_ahead(Token::Comma))),
        declaration_specifier(declarator_rec, assign_expression_rec, resolver),
    );
    single
        .separated_by(just(Token::Comma))
        .collect()
        .then(just(Token::Comma).then(just(Token::Ellipsis)).or_not())
        .map(|(parameters, ellipsis)| ParameterList {
            parameters,
            ellipsis: ellipsis.is_some(),
            empty_is_variadic: true,
        })
        .delimited_by(just(Token::LParen), just(Token::RParen))
}

fn rust_ty<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Spanned<RustTy<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let path = rust_path_with_generic_args(rust_generic_arg_ty())
            .map({
                let resolver = resolver.clone();
                move |path| (resolver.classify_path(&path.0), path.1)
            })
            .filter(|r| {
                let Some(r) = &r.0 else { return false };
                match r.0 {
                    TypeQueryResult::Expr => false,
                    TypeQueryResult::Unsure | TypeQueryResult::Type => true,
                }
            })
            .map(|(resolved, span)| (RustTy::Path((resolved.unwrap().1, span)), span));

        let ptr = just(Token::Star)
            .ignore_then(choice((
                just(Token::Const).to(false),
                mut_token().to(true),
            )))
            .then(rec.clone())
            .map(|(mutable, inner)| RustTy::Ptr {
                mutable: mutable,
                inner: Box::new(inner),
            })
            .map_with(|r, e| (r, e.span()));

        let reference = just(Token::Amp)
            .ignore_then(mut_token().to(true).or_not().map(|m| m.is_some()))
            .then(rec.clone())
            .map(|(mutable, inner)| RustTy::Ref {
                mutable,
                inner: Box::new(inner),
            })
            .map_with(|r, e| (r, e.span()));

        let never = just(Token::Bang)
            .to(RustTy::Never)
            .map_with(|r, e| (r, e.span()));

        let tuple = rec
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(RustTy::Tuple)
            .map_with(|r, e| (r, e.span()));

        let slice_or_array = rec
            .clone()
            .then(
                just(Token::Semicolon)
                    .ignore_then(
                        any()
                            .filter(|t| !matches!(t, Token::RBracket))
                            .map_with(|t, e| (t, e.span()))
                            .repeated()
                            .collect()
                            .map(|tokens| LazyRustConstExpr { tokens }),
                    )
                    .map_with(|r, e| (r, e.span()))
                    .or_not(),
            )
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map(|(inner, len)| {
                if let Some(len) = len {
                    RustTy::Array {
                        inner: Box::new(inner),
                        len,
                    }
                } else {
                    RustTy::Slice(Box::new(inner))
                }
            })
            .map_with(|r, e| (r, e.span()));

        choice((path, ptr, reference, never, tuple, slice_or_array))
    })
}

fn rust_style_function_definition<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let rust_param = identifier()
        .then_ignore(just(Token::Colon))
        .then(rust_ty(resolver.clone()))
        .map({
            let resolver = resolver.clone();
            move |(name, ty)| RustFunctionParam {
                name: (resolver.register_ident(name.0), name.1),
                ty,
            }
        });

    let params = rust_param
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen));

    let ret = just(Token::Arrow)
        .ignore_then(rust_ty(resolver.clone()))
        .or_not()
        .map_with(|ret, e| ret.unwrap_or_else(|| (RustTy::Tuple(vec![]), e.span())));

    fn_token()
        .ignore_then(identifier())
        .then(params)
        .then(ret)
        .then(lazy_compound_statement())
        .map({
            let resolver = resolver.clone();
            move |(((name, params), ret_ty), body)| {
                let name_span = name.1;
                Declaration::FunctionDefinition {
                    signature: FunctionDefinitionSignature::Rust(RustFunctionSignature {
                        name: (resolver.register_ident(name.0), name_span),
                        params,
                        ret_ty,
                    }),
                    body,
                }
            }
        })
}

fn declarator<'src, I, R: TypeResolver>(
    resolver: R,
    assign_expression_rec: impl Parser<
        'src,
        I,
        Spanned<Expression<R>>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let ident = identifier()
            .map({
                let resolver = resolver.clone();
                move |(name, span)| {
                    let ident = resolver.register_ident(name);
                    Declarator::Identifier((ident, span))
                }
            })
            .or(empty().to(Declarator::Abstract))
            .map_with(|r, e| (r, e.span()));
        let grouped = rec
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        let param_list = parameter_type_list(rec, assign_expression_rec, resolver.clone()).map(Err);
        let subscription_resolver = resolver.clone();
        let subscription = lazy_subscription()
            .map(move |subscription| {
                let span = subscription.1;
                (
                    subscription_resolver.register_subscription(subscription),
                    span,
                )
            })
            .map(Ok);

        let direct_declarator = choice((grouped, ident))
            .then(param_list.or(subscription).repeated().collect())
            .map(|(mut base, tails): (_, Vec<_>)| {
                for tail in tails {
                    match tail {
                        Ok(subscription) => {
                            base.0 = Declarator::ArrayDeclarator {
                                declarator: Box::new((base.0, base.1)),
                                subscription,
                            }
                        }
                        Err(param_list) => {
                            base.0 = Declarator::FunctionDeclarator {
                                declarator: Box::new((base.0, base.1)),
                                param_list,
                            }
                        }
                    }
                }
                base
            });

        let declarator = just(Token::Star)
            .ignore_then(type_qualifier().repeated().collect())
            .repeated()
            .collect()
            .then(direct_declarator)
            .map(|(pointers, mut base): (Vec<Vec<_>>, _)| {
                for qualifiers in pointers.into_iter().rev() {
                    base.0 = Declarator::PointerDeclarator {
                        declarator: Box::new((base.0, base.1)),
                        qualifiers,
                    };
                }
                base
            });

        declarator
    })
}

fn struct_declarator<'src, I, R: TypeResolver>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone,
) -> impl Parser<'src, I, Spanned<StructDeclarator<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    declarator_rec
        .then(just(Token::Colon).ignore_then(number()).or_not())
        .map(|(declarator, bits)| StructDeclarator { declarator, bits })
        .map_with(|r, e| (r, e.span()))
}

fn initializer<'src, I, R: TypeResolver>(
    resolver: R,
    stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<Initializer<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|init_rec| {
        let designator = choice((
            expression(assignment_expression(resolver.clone(), stmt_rec.clone()))
                .delimited_by(just(Token::LBracket), just(Token::RBracket))
                .map(Designator::Subscript),
            just(Token::Dot)
                .ignore_then(identifier())
                .map(Designator::Field),
        ))
        .map_with(|r, e| (r, e.span()));

        let initializer_item = designator
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
            .then_ignore(just(Token::Assign))
            .or_not()
            .then(init_rec.clone())
            .map(|(designators, initializer)| InitializerItem {
                designators,
                initializer,
            })
            .map_with(|r, e| (r, e.span()));

        let list = initializer_item
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(Initializer::List)
            .map_with(|r, e| (r, e.span()));

        let expr = assignment_expression(resolver, stmt_rec)
            .map(Initializer::Expr)
            .map_with(|r, e| (r, e.span()));

        choice((list, expr))
    })
}

fn init_declarator_list<'src, I, R: TypeResolver>(
    resolver: R,
    stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Vec<Spanned<InitDeclarator<R>>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    // Per C11 6.2.1p7, the scope of each declared identifier begins at the end of its
    // declarator, before its initializer is evaluated. Process declarators one-by-one,
    // registering each name into the resolver before parsing the corresponding initializer.
    // This also makes each name visible in subsequent initializers within the same declaration
    // (e.g. `int a = x, x = sizeof(x), b = x` sees the inner `x` in `sizeof(x)` and `b = x`).
    custom(move |inp| {
        let mut current_resolver = resolver.clone();
        let mut result = Vec::new();

        loop {
            // Save position before each declarator attempt. On failure (including the
            // empty-declarator case for declarations like `enum E {...};`), we rewind here
            // so the cursor is back to where the caller expects it.
            let declarator_checkpoint = inp.save();
            let item_start = inp.cursor();

            let decl = match inp.parse(
                declarator(
                    current_resolver.clone(),
                    assignment_expression(current_resolver.clone(), stmt_rec.clone()),
                )
                .filter(|d| declarator_has_name(&d.0)),
            ) {
                Ok(d) => d,
                Err(_) => {
                    inp.rewind(declarator_checkpoint);
                    break;
                }
            };

            // Scope of identifier begins at the end of its declarator (C11 6.2.1p7).
            if let Some(ident) = decl.0.ident() {
                current_resolver = current_resolver.declare_ident_as_local(&ident);
            }

            let init: Option<Spanned<Initializer<R>>> = inp.parse(
                just(Token::Assign)
                    .ignore_then(initializer(current_resolver.clone(), stmt_rec.clone()))
                    .or_not(),
            )?;

            let item_span = inp.span_since(&item_start);
            result.push((
                InitDeclarator {
                    declarator: decl,
                    initializer: init,
                },
                item_span,
            ));

            let comma_checkpoint = inp.save();
            if inp.parse(just(Token::Comma)).is_err() {
                inp.rewind(comma_checkpoint);
                break;
            }
        }

        Ok(result)
    })
}

fn declarator_has_name<R: TypeResolver>(decl: &Declarator<R>) -> bool {
    match decl {
        Declarator::Identifier(_) => true,
        Declarator::Abstract => false,
        Declarator::FunctionDeclarator { declarator, .. } => declarator_has_name(&declarator.0),
        Declarator::PointerDeclarator { declarator, .. } => declarator_has_name(&declarator.0),
        Declarator::ArrayDeclarator { declarator, .. } => declarator_has_name(&declarator.0),
    }
}

fn declaration<'src, I, R: TypeResolver>(
    resolver: R,
    stmt_rec: impl Parser<'src, I, Spanned<Statement<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, (Spanned<Declaration<R>>, R), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let expr = assignment_expression(resolver.clone(), stmt_rec.clone());
    let declarator = declarator(resolver.clone(), expr.clone());
    let function = left_recursion(
        declarator
            .clone()
            .filter(|decl| declarator_has_name(&decl.0))
            .filter(|decl| decl.0.is_function())
            .then_ignore(look_ahead(Token::LBrace))
            .then(lazy_compound_statement()),
        declaration_specifier(declarator.clone(), expr.clone(), resolver.clone()),
    )
    .map(
        |(declaration_specifiers, (declarator, body))| Declaration::FunctionDefinition {
            signature: FunctionDefinitionSignature::C {
                declaration_specifiers,
                declarator,
            },
            body,
        },
    );

    let simple = left_recursion(
        init_declarator_list(resolver.clone(), stmt_rec).then_ignore(just(Token::Semicolon)),
        declaration_specifier(declarator, expr, resolver.clone()),
    )
    .map(
        |(declaration_specifiers, declarators)| Declaration::Declaration {
            declaration_specifiers,
            declarators,
        },
    );

    let parser = if resolver.rust_style_syntax_enabled() {
        choice((
            rust_style_function_definition(resolver.clone()),
            function,
            simple,
        ))
        .boxed()
    } else {
        choice((function, simple)).boxed()
    };

    parser.map_with(move |v, e| {
        let nr = resolver.register_decl(&v);
        ((v, e.span()), nr)
    })
}

fn use_item<'src, I>()
-> impl Parser<'src, I, Spanned<UseItem>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    just(Token::Ident("use".to_owned()))
        .ignore_then(
            identifier()
                .separated_by(just(Token::ColonColon))
                .collect()
                .map(|path| UseItem { path }),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|r, e| (r, e.span()))
}

pub fn translation_unit<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Spanned<TranslationUnit<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    #[derive(Clone)]
    enum TranslationUnitItem<R: TypeResolver> {
        Use(Spanned<UseItem>),
        Declaration(Spanned<Declaration<R>>),
        Empty,
    }

    custom(move |inp| {
        let mut current_resolver = resolver.clone();
        let mut items = Vec::new();

        loop {
            let checkpoint = inp.save();
            if inp.next().is_none() {
                inp.rewind(checkpoint);
                break;
            }
            inp.rewind(checkpoint);

            let item = if let Ok(item) = inp.parse(use_item()) {
                TranslationUnitItem::Use(item)
            } else if let Ok((decl, next_resolver)) =
                inp.parse(declaration(current_resolver.clone(), statement(current_resolver.clone())))
            {
                current_resolver = next_resolver;
                TranslationUnitItem::Declaration(decl)
            } else {
                inp.parse(just(Token::Semicolon))?;
                TranslationUnitItem::Empty
            };
            items.push(item);
        }

        Ok(items)
    })
        .map_with(|items, e| {
            let mut rust_use_items = Vec::new();
            let mut declarations = Vec::new();
            for item in items {
                match item {
                    TranslationUnitItem::Use(item) => rust_use_items.push(item),
                    TranslationUnitItem::Declaration(item) => declarations.push(item),
                    TranslationUnitItem::Empty => {}
                }
            }
            (
                TranslationUnit {
                    rust_use_items,
                    items: declarations,
                },
                e.span(),
            )
        })
}

#[test]
fn parser_is_constructible() {
    use chumsky::input::Input;
    let parser = translation_unit(crate::StatelessResolver::new());
    parser.parse((&[]).map(
        Span::new(0, 1..2),
        |_| unreachable!(),
    ));
}
