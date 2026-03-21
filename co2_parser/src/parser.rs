use chumsky::{
    input::{SliceInput, ValueInput},
    prelude::*,
};
use co2_ast::TypeResolver;
use co2_ast::*;

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
        (
            LazyCompoundStatement {
                tokens: (<[_]>::to_vec(e.slice()), e.span()),
            },
            e.span(),
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
        inp: &mut chumsky::input::InputRef<'src, '_, I, extra::Full<Rich<'src, Token>, (), ()>>,
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

        let compound = compound_statement(resolver, stmt_rec.clone()).map(Statement::Compound);

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

        let for_statement = just(Token::For)
            .ignore_then(
                expression_rec
                    .clone()
                    .or_not()
                    .then_ignore(just(Token::Semicolon))
                    .then(
                        expression_rec
                            .clone()
                            .or_not()
                            .then_ignore(just(Token::Semicolon)),
                    )
                    .then(expression_rec.clone().or_not())
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then(stmt_rec.clone())
            .map(|(((init, cond), post), body)| Statement::For {
                init,
                cond,
                post,
                body: Box::new(body),
            });

        choice((
            if_statement,
            while_statement,
            do_while_statement,
            for_statement,
            switch_statement,
            case_statement,
            default_statement,
            labeled_statement,
            goto_statement,
            break_statement,
            continue_statement,
            jump_statement,
            expression_statement,
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

fn assignment_expression<'src, I, R: TypeResolver>(
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

        let primary_expression = choice((
            compound_literal,
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
            rust_path()
                .map({
                    let resolver = resolver.clone();
                    move |path| (resolver.classify_path(&path.0), path.1)
                })
                .filter(|r| {
                    let Some(r) = &r.0 else { return false };
                    match r.0 {
                        TypeQueryResult::Type => false,
                        TypeQueryResult::Unsure | TypeQueryResult::Expr => true,
                    }
                })
                .map(|r| Expression::Identifier((r.0.unwrap().1, r.1))),
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
                .map(|x: (Expression<R>, SimpleSpan)| x.0),
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

        let conditional_expr = logical_or_expr
            .clone()
            .then(
                just(Token::Question)
                    .ignore_then(rec.clone())
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

fn parse_integer_constant(text: &str) -> i64 {
    parse_unsigned_integer_constant(text)
        .map(|v| v as i64)
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
        (
            LazySubscription {
                tokens: <[_]>::to_vec(e.slice()),
            },
            e.span(),
        )
    })
}

fn rust_path<'src, I>()
-> impl Parser<'src, I, Spanned<RustPath>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let generics = rec
            .separated_by(just(Token::Comma))
            .collect()
            .map(RustPathSegment::Generics)
            .delimited_by(just(Token::Lt), just(Token::Gt))
            .map_with(|r, e| (r, e.span()));

        identifier()
            .then(generics.or_not())
            .map(|(ident, generics)| {
                let mut segments = vec![(RustPathSegment::Ident(ident.0), ident.1)];
                if let Some(generics) = generics {
                    segments.push(generics);
                }
                segments
            })
            .separated_by(just(Token::ColonColon))
            .at_least(1)
            .collect::<Vec<Vec<Spanned<RustPathSegment>>>>()
            .map(|parts| RustPath {
                segments: parts
                    .into_iter()
                    .flatten()
                    .collect::<Vec<Spanned<RustPathSegment>>>(),
            })
            .map_with(|r, e| (r, e.span()))
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
        type_specifier_rec,
    )
    .map(|(specifiers, declarators)| StructOrUnionField {
        specifiers,
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
            .map(|r| TypeSpecifier::TypedefName((r.0.unwrap().1, r.1))))
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
    ))
    .map_with(|r, e| (r, e.span()))
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
    .map_with(|r, e| (r, e.span()))
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
    .map_with(|r, e| (r, e.span()))
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
        })
        .delimited_by(just(Token::LParen), just(Token::RParen))
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

        let param_list = parameter_type_list(rec, assign_expression_rec, resolver).map(Err);
        let subscription = lazy_subscription().map(Ok);

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
    declarator(
        resolver.clone(),
        assignment_expression(resolver.clone(), stmt_rec.clone()),
    )
    .filter(|decl| declarator_has_name(&decl.0))
    .then(
        just(Token::Assign)
            .ignore_then(initializer(resolver, stmt_rec.clone()))
            .or_not(),
    )
    .map(|(declarator, initializer)| InitDeclarator {
        declarator,
        initializer,
    })
    .map_with(|r, e| (r, e.span()))
    .separated_by(just(Token::Comma))
    .collect()
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
            declaration_specifiers,
            declarator,
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

    choice((function, simple)).map_with(move |v, e| {
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
    use_item()
        .repeated()
        .collect()
        .then(
            declaration(resolver.clone(), statement(resolver.clone()))
                .map(|x| x.0)
                .repeated()
                .collect(),
        )
        .map_with(|(rust_use_items, items), e| {
            (
                TranslationUnit {
                    rust_use_items,
                    items,
                },
                e.span(),
            )
        })
}

#[test]
fn parser_is_constructible() {
    use chumsky::input::Input;
    let parser = translation_unit(&crate::StatelessResolver);
    parser.parse((&[]).map(
        SimpleSpan {
            start: 1,
            end: 2,
            context: (),
        },
        |_| unreachable!(),
    ));
}
