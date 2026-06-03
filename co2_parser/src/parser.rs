use chumsky::{
    input::{SliceInput, ValueInput},
    prelude::*,
};
use co2_ast::TypeResolver;
use co2_ast::{
    BinOp, CompoundStatement, Constant, Declaration, DeclarationSpecifier, Declarator, Designator,
    EnumSpecifier, Enumerator, Expression, FileId, ForInit, FunctionDefinitionSignature,
    FunctionSpecifier, GenericAssociation, InitDeclarator, Initializer, InitializerItem,
    LazyCompoundStatement, LazyRustConstExpr, LazySubscription, ModItem, ParameterList,
    RustAttribute, RustAttributeStyle, RustFunctionParam, RustFunctionSignature, RustPath,
    RustPathSegment, RustStructField, RustTy, Span, Spanned, SpecifierQualifier, StatelessResolver,
    Statement, StatementOrDeclaration, StorageClassSpecifier, StringLiteral, StringLiteralPrefix,
    StructDeclarator, StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, Token,
    TranslationUnit, TypeName, TypeQualifier, TypeQueryResult, TypeSpecifier, UnaryOp, UpdateOp,
    UseItem, parse_unsigned_integer_constant,
};

fn join_spans(start: Span, end: Span) -> Span {
    let start_data = start.data();
    let end_data = end.data();
    if start_data.context == end_data.context {
        Span::from_parts(start_data.context, start_data.start..end_data.end)
    } else {
        start
    }
}

fn merge_string_literals(parts: Vec<StringLiteral>) -> StringLiteral {
    let prefix = parts
        .iter()
        .map(|part| part.prefix)
        .find(|prefix| *prefix != StringLiteralPrefix::None)
        .unwrap_or(StringLiteralPrefix::None);
    debug_assert!(
        parts
            .iter()
            .all(|part| part.prefix == StringLiteralPrefix::None || part.prefix == prefix)
    );
    StringLiteral {
        prefix,
        bytes: parts.into_iter().flat_map(|part| part.bytes).collect(),
    }
}

fn single_token_span(slice: &[Spanned<Token>], fallback: Span) -> Span {
    slice.first().map_or(fallback, |(_, span)| *span)
}

fn slice_span(slice: &[Spanned<Token>], fallback: Span) -> Span {
    slice
        .first()
        .zip(slice.last())
        .map_or(fallback, |(first, last)| join_spans(first.1, last.1))
}

fn attr_content<'src, I>()
-> impl Parser<'src, I, Spanned<Vec<Spanned<Token>>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let inner = recursive(|content| {
        let any_non_delim = any()
            .filter(|t| {
                !matches!(
                    t,
                    Token::LParen
                        | Token::RParen
                        | Token::LBracket
                        | Token::RBracket
                        | Token::LBrace
                        | Token::RBrace
                )
            })
            .ignored();
        let any_group = choice((
            content
                .clone()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .ignored(),
            content
                .clone()
                .delimited_by(just(Token::LBracket), just(Token::RBracket))
                .ignored(),
            content
                .clone()
                .delimited_by(just(Token::LBrace), just(Token::RBrace))
                .ignored(),
        ));

        choice((any_non_delim, any_group)).repeated().ignored()
    });

    inner
        .delimited_by(just(Token::LBracket), just(Token::RBracket))
        .map_with(|(), e| {
            let slice: &[Spanned<Token>] = e.slice();
            let inner = if slice.len() >= 2 {
                slice[1..slice.len() - 1].to_vec()
            } else {
                Vec::new()
            };
            (inner, slice_span(slice, e.span()))
        })
}

fn parse_rust_attr(
    tokens: Vec<Spanned<Token>>,
    span: Span,
) -> Result<RustAttribute, Rich<'static, Token, Span>> {
    let mut idx = 0;
    let mut path = Vec::new();
    while let Some((token, token_span)) = tokens.get(idx) {
        let Token::Ident(segment) = token else {
            break;
        };
        path.push((segment.clone(), *token_span));
        idx += 1;
        if !matches!(tokens.get(idx), Some((Token::ColonColon, _))) {
            break;
        }
        idx += 1;
    }
    if path.is_empty() {
        return Err(Rich::custom(
            span,
            "attribute path must start with an identifier",
        ));
    }
    Ok(RustAttribute {
        path,
        args: tokens[idx..].to_vec(),
        style: RustAttributeStyle::Outer,
    })
}

fn rust_attr<'src, I>()
-> impl Parser<'src, I, Spanned<RustAttribute>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    just(Token::Hash)
        .ignore_then(attr_content())
        .try_map(|(tokens, span), _| parse_rust_attr(tokens, span).map(|attr| (attr, span)))
}

fn doc_attr<'src, I>()
-> impl Parser<'src, I, Spanned<RustAttribute>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! {
        Token::DocComment { inner, text } => (inner, text),
    }
    .map_with(|(inner, text), e| {
        let span = e.span();
        (
            RustAttribute {
                path: vec![("doc".to_owned(), span)],
                args: vec![(
                    Token::StringLit(StringLiteral {
                        prefix: StringLiteralPrefix::None,
                        bytes: text.into_bytes(),
                    }),
                    span,
                )],
                style: if inner {
                    RustAttributeStyle::Inner
                } else {
                    RustAttributeStyle::Outer
                },
            },
            span,
        )
    })
}

fn rust_attrs<'src, I>()
-> impl Parser<'src, I, Vec<Spanned<RustAttribute>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice((rust_attr(), doc_attr())).repeated().collect()
}

fn rust_path_span<R: TypeResolver>(path: &RustPath<R>, fallback: Span) -> Span {
    path.segments
        .first()
        .zip(path.segments.last())
        .map_or(fallback, |(first, last)| join_spans(first.1, last.1))
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
    .map_with(|(), e| {
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
    custom(move |inp| {
        let before = inp.cursor();
        inp.parse(just(Token::LBrace))?;
        let statements = inp.parse(repeated_statement_with_modified_resolver(resolver.clone()))?;
        inp.parse(just(Token::RBrace))?;
        Ok((CompoundStatement { statements }, inp.span_since(&before)))
    })
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
    custom(move |inp| {
        let before = inp.cursor();
        let checkpoint = inp.save();
        let first = inp.next();
        let prefer_declaration = match first {
            Some(
                Token::Typedef
                | Token::Extern
                | Token::Static
                | Token::Constexpr
                | Token::Atomic
                | Token::Auto
                | Token::Register
                | Token::Inline
                | Token::Const
                | Token::Restrict
                | Token::Volatile
                | Token::Struct
                | Token::Union
                | Token::Enum
                | Token::Int
                | Token::Bool
                | Token::Void
                | Token::Char
                | Token::Short
                | Token::Long
                | Token::Float
                | Token::Double
                | Token::Signed
                | Token::Unsigned
                | Token::Typeof,
            ) => true,
            Some(Token::Ident(_)) => {
                inp.rewind(checkpoint.clone());
                let prefer = inp
                    .parse(rust_path())
                    .ok()
                    .as_ref()
                    .and_then(|path| resolver.classify_path(&path.0).ok())
                    .is_some_and(|(result, _)| {
                        matches!(result, TypeQueryResult::Unsure | TypeQueryResult::Type)
                    });
                inp.rewind(checkpoint.clone());
                prefer
            }
            Some(Token::ColonColon) => {
                inp.rewind(checkpoint.clone());
                let prefer = inp
                    .parse(rust_path())
                    .ok()
                    .as_ref()
                    .and_then(|path| resolver.classify_path(&path.0).ok())
                    .is_some_and(|(result, _)| matches!(result, TypeQueryResult::Type));
                inp.rewind(checkpoint.clone());
                prefer
            }
            _ => false,
        };
        inp.rewind(checkpoint);

        let (value, next_resolver) = if prefer_declaration {
            inp.parse(
                declaration(resolver.clone(), stmt_rec.clone())
                    .map(|(v, r)| (StatementOrDeclaration::Declaration(v), r)),
            )?
        } else {
            inp.parse(
                stmt_rec
                    .clone()
                    .map(|v| (StatementOrDeclaration::Statement(v), resolver.clone())),
            )?
        };

        Ok(((value, inp.span_since(&before)), next_resolver))
    })
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
            .ignore_then(
                just(Token::Star)
                    .ignore_then(expression_rec.clone())
                    .map(Statement::IndirectGoto)
                    .or(identifier().map(Statement::Goto)),
            )
            .then_ignore(just(Token::Semicolon));
        let break_statement = just(Token::Break).ignore_then(
            just(Token::Ident("co2".to_string()))
                .then_ignore(just(Token::Semicolon))
                .to(Statement::BreakCo2)
                .or(just(Token::Semicolon).to(Statement::Break)),
        );
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
            .map_with(|_, e| e.span())
            .then(just(Token::Colon).ignore_then(stmt_rec.clone()))
            .map(|(keyword_span, statement)| Statement::Default {
                keyword_span,
                statement: Box::new(statement),
            });

        let expression_statement = expression_rec
            .clone()
            .map(Statement::Expression)
            .then_ignore(just(Token::Semicolon));
        let empty_statement = just(Token::Semicolon).to(Statement::Empty);

        let compound =
            compound_statement(resolver.clone(), stmt_rec.clone()).map(Statement::Compound);

        let if_statement = custom({
            let expression_rec = expression_rec.clone();
            let stmt_rec = stmt_rec.clone();
            move |inp| {
                inp.parse(just(Token::If))?;
                let cond = inp.parse(
                    expression_rec
                        .clone()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                )?;
                let then_branch = inp.parse(stmt_rec.clone())?;
                let else_branch = {
                    let checkpoint = inp.save();
                    let has_else = inp.next() == Some(Token::Else);
                    inp.rewind(checkpoint);
                    if has_else {
                        inp.parse(just(Token::Else))?;
                        Some(inp.parse(stmt_rec.clone())?)
                    } else {
                        None
                    }
                };
                Ok(Statement::If {
                    cond,
                    then_branch: Box::new(then_branch),
                    else_branch: else_branch.map(Box::new),
                })
            }
        });

        let while_statement = custom({
            let expression_rec = expression_rec.clone();
            let stmt_rec = stmt_rec.clone();
            move |inp| {
                inp.parse(just(Token::While))?;
                let cond = inp.parse(
                    expression_rec
                        .clone()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                )?;
                let body = inp.parse(stmt_rec.clone())?;
                Ok(Statement::While {
                    cond,
                    body: Box::new(body),
                })
            }
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
                        (expr.map(ForInit::Expression), init_resolver)
                    }
                };

                let loop_expr = expression(assignment_expression(
                    loop_resolver.clone(),
                    stmt_rec.clone(),
                ));
                let cond = inp.parse(
                    loop_expr
                        .clone()
                        .or_not()
                        .then_ignore(just(Token::Semicolon)),
                )?;
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

        custom(move |inp| {
            let before = inp.cursor();
            let checkpoint = inp.save();
            let first = inp.next();
            inp.rewind(checkpoint);

            let stmt = match first {
                Some(Token::If) => inp.parse(if_statement.clone())?,
                Some(Token::While) => inp.parse(while_statement.clone())?,
                Some(Token::Do) => inp.parse(do_while_statement.clone())?,
                Some(Token::For) => inp.parse(for_statement.clone())?,
                Some(Token::Switch) => inp.parse(switch_statement.clone())?,
                Some(Token::Case) => inp.parse(case_statement.clone())?,
                Some(Token::Default) => inp.parse(default_statement.clone())?,
                Some(Token::Goto) => inp.parse(goto_statement.clone())?,
                Some(Token::Break) => inp.parse(break_statement.clone())?,
                Some(Token::Continue) => inp.parse(continue_statement.clone())?,
                Some(Token::Return) => inp.parse(jump_statement.clone())?,
                Some(Token::Semicolon) => inp.parse(empty_statement.clone())?,
                Some(Token::LBrace) => inp.parse(compound.clone())?,
                _ => inp.parse(labeled_or_expression_statement.clone())?,
            };

            Ok((stmt, inp.span_since(&before)))
        })
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
    assignment
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
        })
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
                    .then_ignore(just(Token::Ellipsis))
                    .then(rec.clone())
                    .delimited_by(just(Token::LBracket), just(Token::RBracket))
                    .map(|(start, end)| Designator::Range(start, end)),
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
            just(Token::VaCopy).ignore_then(
                rec.clone()
                    .then_ignore(just(Token::Comma))
                    .then(rec.clone())
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .map(|(dest, src)| {
                        let dest = Box::new(dest);
                        let src = Box::new(src);
                        Expression::VaCopy { dest, src }
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
                .to(Expression::Constant(Constant::Float(f64::INFINITY, co2_ast::FloatSuffix::None))),
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
                .to(Expression::Constant(Constant::Float(f64::NAN, co2_ast::FloatSuffix::None))),
            just(Token::And)
                .ignore_then(identifier())
                .map(Expression::LabelAddress),
            expression(rec.clone())
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .map(|x: Spanned<Expression<R>>| x.0),
            just(Token::LParen)
                .ignore_then(compound_statement(resolver.clone(), stmt_rec.clone()))
                .then_ignore(just(Token::RParen))
                .map(|body| Expression::GnuStatementExpr {
                    body: Box::new(body),
                }),
            rust_path_expr_simple().try_map({
                let resolver = resolver.clone();
                move |path, _| {
                    let path_span = rust_path_span(&path.0, path.1);
                    match resolver.classify_path(&path.0) {
                    Ok((TypeQueryResult::Unsure | TypeQueryResult::Expr, resolved)) => {
                        Ok(Expression::Identifier((resolved, path_span)))
                    }
                    Ok((TypeQueryResult::Type, _)) => {
                        Err(Rich::custom(path_span, "expected expression, found type name"))
                    }
                    Err((msg, span)) => Err(Rich::custom(span, msg)),
                    }
                }
            }),
            select! {
                Token::StringLit(s) => s,
            }
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|parts| Expression::Constant(Constant::String(merge_string_literals(parts)))),
            select! {
                Token::Integer(i, suffix) => Expression::Constant(Constant::Int(parse_integer_constant(&i), suffix)),
                Token::FloatLit(i, suffix) => Expression::Constant(Constant::Float(parse_float_constant(&i), suffix)),
                Token::CharLit(s) => {
                    let ch = s.first().copied().expect("empty char literal");
                    Expression::Constant(Constant::Char(u32::from(ch)))
                },
            },
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

                let alignof_type_expression = just(Token::Alignof)
                    .ignore_then(
                        type_name(resolver.clone(), rec.clone())
                            .delimited_by(just(Token::LParen), just(Token::RParen)),
                    )
                    .map(|ty| Expression::AlignofType(Box::new(ty)))
                    .map_with(|r, e| (r, e.span()));

                let offsetof_expression = just(Token::Offsetof)
                    .ignore_then(just(Token::LParen))
                    .ignore_then(type_name(resolver.clone(), rec.clone()))
                    .then_ignore(just(Token::Comma))
                    .then(select! { Token::Ident(s) => s }.map_with(|s, e| (s, e.span())))
                    .then_ignore(just(Token::RParen))
                    .map(|(ty, (field, field_span))| Expression::Offsetof {
                        ty: Box::new(ty),
                        field,
                        field_span,
                    })
                    .map_with(|r, e| (r, e.span()));

                let types_compatible_p_expression = just(Token::BuiltinTypesCompatibleP)
                    .ignore_then(just(Token::LParen))
                    .ignore_then(type_name(resolver.clone(), rec.clone()))
                    .then_ignore(just(Token::Comma))
                    .then(type_name(resolver.clone(), rec.clone()))
                    .then_ignore(just(Token::RParen))
                    .map(|(ty1, ty2)| Expression::BuiltinTypesCompatibleP {
                        ty1: Box::new(ty1),
                        ty2: Box::new(ty2),
                    })
                    .map_with(|r, e| (r, e.span()));

                let constant_p_expression = just(Token::BuiltinConstantP)
                    .ignore_then(
                        rec.clone()
                            .map(|expr| Expression::BuiltinConstantP {
                                expr: Box::new(expr),
                            })
                            .delimited_by(just(Token::LParen), just(Token::RParen)),
                    )
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
                    .ignore_then(unary.clone())
                    .map(|expr| Expression::Sizeof(Box::new(expr)))
                    .map_with(|r, e| (r, e.span()));

                let alignof_unary_expression = just(Token::Alignof)
                    .ignore_then(unary)
                    .map(|expr| Expression::Alignof(Box::new(expr)))
                    .map_with(|r, e| (r, e.span()));

                choice((
                    sizeof_type_expression,
                    alignof_type_expression,
                    offsetof_expression,
                    constant_p_expression,
                    types_compatible_p_expression,
                    prefix_inc_expression,
                    prefix_dec_expression,
                    unary_operator_expression,
                    sizeof_unary_expression,
                    alignof_unary_expression,
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

        conditional_expr
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
            .map_with(|r, e| (r.0, e.span()))
    })
}

fn parse_integer_constant(text: &str) -> i128 {
    parse_unsigned_integer_constant(text)
        .map_or_else(|| panic!("invalid integer literal `{text}`"), |v| v as i128)
}

fn parse_float_constant(text: &str) -> f64 {
    text.parse::<f64>()
        .ok()
        .or_else(|| parse_hex_float_constant(text))
        .unwrap_or_else(|| panic!("invalid float literal `{text}`"))
}

fn parse_hex_float_constant(text: &str) -> Option<f64> {
    let (significand, exponent) = text.split_once(['p', 'P'])?;
    let exponent = exponent.parse::<i32>().ok()?;
    let significand = significand
        .strip_prefix("0x")
        .or_else(|| significand.strip_prefix("0X"))?;
    let (int_part, frac_part) = significand.split_once('.').unwrap_or((significand, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }

    let mut value = 0.0f64;
    for ch in int_part.chars() {
        let digit = ch.to_digit(16)?;
        value = value * 16.0 + f64::from(digit);
    }
    let mut scale = 1.0f64 / 16.0;
    for ch in frac_part.chars() {
        let digit = ch.to_digit(16)?;
        value += f64::from(digit) * scale;
        scale /= 16.0;
    }
    Some(value * 2.0f64.powi(exponent))
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
    .map_with(|(), e| {
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

fn rust_path_with_generic_args<'src, I, R: TypeResolver>(
    generic_ty: impl Parser<'src, I, Spanned<RustTy<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<RustPath<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
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

    just(Token::ColonColon).or_not().ignore_then(
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
        .collect::<Vec<Vec<Spanned<RustPathSegment<R>>>>>()
        .map(|parts| {
            let segments = parts
                .into_iter()
                .flatten()
                .collect::<Vec<Spanned<RustPathSegment<R>>>>();
            let span = segments
                .first()
                .zip(segments.last())
                .map_or(Span::from_parts(FileId::INVALID, 0..0), |(first, last)| {
                    join_spans(first.1, last.1)
                });
            (RustPath { segments }, span)
        }),
    )
}

fn rust_path<'src, I>()
-> impl Parser<'src, I, Spanned<RustPath<StatelessResolver>>, extra::Err<Rich<'src, Token, Span>>>
+ Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_path_with_generic_args(rust_generic_arg_ty())
}

/// Like `rust_path` but only allows generic arguments after `::` (turbofish syntax).
/// Bare `<...>` after an identifier is NOT parsed as generics.
/// Used in expression context to avoid ambiguity with `<` as less-than operator.
fn rust_path_expr<'src, I, R: TypeResolver>(
    generic_ty: impl Parser<'src, I, Spanned<RustTy<R>>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<RustPath<R>>, extra::Err<Rich<'src, Token, Span>>> + Clone
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

    just(Token::ColonColon).or_not().ignore_then(
        choice((
            identifier().map(|ident| vec![(RustPathSegment::Ident(ident.0), ident.1)]),
            generics.map(|generics| vec![generics]),
        ))
        .separated_by(just(Token::ColonColon))
        .at_least(1)
        .collect::<Vec<Vec<Spanned<RustPathSegment<R>>>>>()
        .map(|parts| {
            let segments = parts
                .into_iter()
                .flatten()
                .collect::<Vec<Spanned<RustPathSegment<R>>>>();
            let span = segments
                .first()
                .zip(segments.last())
                .map_or(Span::from_parts(FileId::INVALID, 0..0), |(first, last)| {
                    join_spans(first.1, last.1)
                });
            (RustPath { segments }, span)
        }),
    )
}

fn rust_path_expr_simple<'src, I>()
-> impl Parser<'src, I, Spanned<RustPath<StatelessResolver>>, extra::Err<Rich<'src, Token, Span>>>
+ Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_path_expr(rust_generic_arg_ty())
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
            .ignore_then(choice((just(Token::Const).to(false), mut_token().to(true))))
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
    custom(move |inp| {
        let mut elems = vec![inp.parse(left_elem.clone())?];
        loop {
            let checkpoint = inp.save();
            if let Ok(base_value) = inp.parse(base.clone()) {
                return Ok((elems, base_value));
            }
            inp.rewind(checkpoint);
            elems.push(inp.parse(left_elem.clone())?);
        }
    })
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
    let normal = left_recursion(
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

    // GCC extension: accept bare `;` as an empty struct/union member declaration.
    let bare_semicolon = just(Token::Semicolon).map_with(|_, e| {
        let span = e.span();
        (
            StructOrUnionField {
                specifiers: vec![],
                declarators: vec![],
            },
            span,
        )
    });

    let single = choice((normal, bare_semicolon));
    single
        .repeated()
        .collect()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .map(|fields: Vec<Spanned<StructOrUnionField<R>>>| {
            fields
                .into_iter()
                .filter(|(field, _)| {
                    // Filter out empty declarations: GCC allows `int;` and bare `;`
                    // which should not produce any field. Anonymous struct/union members
                    // have a struct-or-union specifier and must be kept.
                    if field.specifiers.is_empty() && field.declarators.is_empty() {
                        return false;
                    }
                    if !field.declarators.is_empty()
                        && field.declarators.iter().all(|(d, _)| {
                            matches!(d.declarator.0, Declarator::Abstract) && d.bits.is_none()
                        })
                    {
                        let has_anon_struct_union = field.specifiers.iter().any(|(s, _)| {
                            matches!(
                                s,
                                SpecifierQualifier::TypeSpecifier((
                                    TypeSpecifier::StructOrUnion { .. },
                                    _,
                                ))
                            )
                        });
                        return has_anon_struct_union;
                    }
                    true
                })
                .collect()
        })
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
                struct_or_union_fields(rec.clone(), declarator_rec)
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
                    .ignore_then(assign_expression_rec.clone())
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

        let typeof_declarator = declarator(resolver.clone(), assign_expression_rec.clone());
        let typeof_specifier_qualifier = choice((
            rec.clone().map(SpecifierQualifier::TypeSpecifier),
            type_qualifier().map(SpecifierQualifier::TypeQualifier),
        ))
        .map_with(|r, e| (r, e.span()));
        let typeof_type_name = typeof_specifier_qualifier
            .repeated()
            .at_least(1)
            .collect::<Vec<_>>()
            .then(typeof_declarator.or_not())
            .map(|(specifier_qualifier_list, abstract_declarator)| TypeName {
                specifier_qualifier_list,
                abstract_declarator: abstract_declarator.and_then(|decl| {
                    if matches!(decl.0, Declarator::Abstract) {
                        None
                    } else {
                        Some(decl)
                    }
                }),
            });

        let typeof_type_specifier = just(Token::Typeof)
            .ignore_then(just(Token::LParen))
            .ignore_then(choice((
                typeof_type_name.map(|type_name| TypeSpecifier::TypeofType(Box::new(type_name))),
                assign_expression_rec
                    .clone()
                    .map(|expr| TypeSpecifier::TypeofExpr(Box::new(expr))),
            )))
            .then_ignore(just(Token::RParen));

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
        .or(typeof_type_specifier)
        .or(rust_path().try_map({
            let resolver = resolver.clone();
            move |path, _| {
                let path_span = rust_path_span(&path.0, path.1);
                match resolver.classify_path(&path.0) {
                    Ok((TypeQueryResult::Unsure | TypeQueryResult::Type, resolved)) => {
                        Ok(TypeSpecifier::TypedefName((resolved, path_span)))
                    }
                    Ok((TypeQueryResult::Expr, _)) => Err(Rich::custom(
                        path_span,
                        "expected type name, found expression",
                    )),
                    Err((msg, span)) => Err(Rich::custom(span, msg)),
                }
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
        just(Token::Constexpr).to(StorageClassSpecifier::Constexpr),
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
        just(Token::Atomic).to(TypeQualifier::Atomic),
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

fn type_token<'src, I>() -> impl Parser<'src, I, (), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! { Token::Ident(s) if s == "type" => () }.labelled("type")
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
    // Like `left_recursion`, but with a typedef-name disambiguation rule: when no
    // type specifier has been accumulated yet and the next token is a typedef name,
    // the token must be consumed as a type specifier rather than as a declarator
    // identifier.  Without this check `const T[5]` (T a typedef) misparsed as
    // declaration-specifier=["const"], declarator=T[5], leaving no type specifier.
    let single = custom({
        let resolver = resolver.clone();
        let declarator_rec = declarator_rec.clone();
        let assign_expression_rec = assign_expression_rec.clone();
        move |inp| {
            let make_spec = || {
                declaration_specifier(
                    declarator_rec.clone(),
                    assign_expression_rec.clone(),
                    resolver.clone(),
                )
            };
            let try_decl = || {
                declarator_rec
                    .clone()
                    .then_ignore(look_ahead(Token::RParen).or(look_ahead(Token::Comma)))
            };

            let mut specs = vec![inp.parse(make_spec())?];
            loop {
                let has_type_spec = specs
                    .iter()
                    .any(|(s, _)| matches!(s, DeclarationSpecifier::TypeSpecifier(_)));

                // If we have no type specifier yet, peek at the next token.  If it is a
                // typedef name it must become the type specifier, not a declarator ident.
                let next_is_typedef_name = if has_type_spec {
                    false
                } else {
                    let checkpoint = inp.save();
                    let next = inp.next();
                    inp.rewind(checkpoint);
                    match next {
                        Some(Token::Ident(s)) => matches!(
                            resolver.classify_path(&RustPath::<StatelessResolver>::from_ident((
                                s,
                                Span::from_parts(FileId::INVALID, 0..0)
                            ))),
                            Ok((TypeQueryResult::Type | TypeQueryResult::Unsure, _))
                        ),
                        _ => false,
                    }
                };

                if !next_is_typedef_name {
                    let checkpoint = inp.save();
                    if let Ok(decl) = inp.parse(try_decl()) {
                        return Ok((specs, decl));
                    }
                    inp.rewind(checkpoint);
                }

                specs.push(inp.parse(make_spec())?);
            }
        }
    });
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
                move |path| {
                    let span = rust_path_span(&path.0, path.1);
                    (resolver.classify_path(&path.0), span)
                }
            })
            .filter(|(result, _)| {
                let Ok(r) = result else { return false };
                match r.0 {
                    TypeQueryResult::Expr => false,
                    TypeQueryResult::Unsure | TypeQueryResult::Type => true,
                }
            })
            .map(|(resolved, span)| (RustTy::Path((resolved.unwrap().1, span)), span));

        let ptr = just(Token::Star)
            .ignore_then(choice((just(Token::Const).to(false), mut_token().to(true))))
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

fn rust_style_type_definition_with_attrs<'src, I, R: TypeResolver>(
    resolver: R,
    attrs: Vec<Spanned<RustAttribute>>,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let pub_token = just(Token::Ident("pub".to_string()))
        .or_not()
        .map(|opt| opt.is_some());

    pub_token
        .then(type_token())
        .map(|(is_pub, ())| is_pub)
        .then(identifier())
        .then_ignore(just(Token::Assign))
        .then(rust_ty(resolver.clone()))
        .map({
            let resolver = resolver.clone();
            move |((is_pub, name), ty)| {
                let name_span = name.1;
                Declaration::RustTypeAlias {
                    attrs: attrs.clone(),
                    ident: (resolver.register_ident(name.0), name_span),
                    ty,
                    is_pub,
                }
            }
        })
}

fn rust_style_struct_definition_with_attrs<'src, I, R: TypeResolver>(
    resolver: R,
    attrs: Vec<Spanned<RustAttribute>>,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let rust_struct_field = identifier()
        .then_ignore(just(Token::Colon))
        .then(rust_ty(resolver.clone()))
        .map({
            let resolver = resolver.clone();
            move |(name, ty)| RustStructField {
                name: (resolver.register_ident(name.0), name.1),
                ty,
            }
        });

    let fields = rust_struct_field
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace));

    just(Token::Ident("pub".to_string()))
        .ignore_then(just(Token::Struct))
        .ignore_then(identifier())
        .then(fields)
        .map({
            let resolver = resolver.clone();
            move |(name, fields)| {
                let name_span = name.1;
                Declaration::RustStruct {
                    attrs: attrs.clone(),
                    ident: (resolver.register_ident(name.0), name_span),
                    fields,
                    is_pub: true,
                }
            }
        })
}

fn rust_style_function_definition_with_attrs<'src, I, R: TypeResolver>(
    resolver: R,
    attrs: Vec<Spanned<RustAttribute>>,
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

    let pub_token = just(Token::Ident("pub".to_string()))
        .or_not()
        .map(|opt| opt.is_some());

    pub_token
        .then(fn_token())
        .map(|(is_pub, ())| is_pub)
        .then(identifier())
        .then(params)
        .then(ret)
        .then(lazy_compound_statement())
        .map({
            let resolver = resolver.clone();
            move |((((is_pub, name), params), ret_ty), body)| {
                let name_span = name.1;
                Declaration::FunctionDefinition {
                    attrs: Vec::new(),
                    signature: FunctionDefinitionSignature::Rust(RustFunctionSignature {
                        attrs: attrs.clone(),
                        name: (resolver.register_ident(name.0), name_span),
                        params,
                        ret_ty,
                        is_pub,
                    }),
                    body,
                }
            }
        })
}

fn rust_style_type_definition<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_style_type_definition_with_attrs(resolver, Vec::new())
}

fn rust_style_struct_definition<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_style_struct_definition_with_attrs(resolver, Vec::new())
}

fn rust_style_function_definition<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, Declaration<R>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    rust_style_function_definition_with_attrs(resolver, Vec::new())
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
                    (Declarator::Identifier((ident, span)), span)
                }
            })
            .or(empty().map_with(|(), e| (Declarator::Abstract, e.span())));
        let grouped = rec
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        let param_list = parameter_type_list(rec, assign_expression_rec, resolver.clone())
            .map_with(|param_list, e| (param_list, slice_span(e.slice(), e.span())))
            .map(Err);
        let subscription_resolver = resolver.clone();
        let subscription = lazy_subscription()
            .map(move |subscription| {
                let span = subscription.1;
                (
                    (
                        subscription_resolver.register_subscription(subscription),
                        span,
                    ),
                    span,
                )
            })
            .map(Ok);

        let direct_declarator = choice((grouped, ident))
            .then(param_list.or(subscription).repeated().collect())
            .map(|(mut base, tails): (_, Vec<_>)| {
                for tail in tails {
                    let base_span = base.1;
                    let has_placeholder_span = matches!(base.0, Declarator::Abstract);
                    match tail {
                        Ok((subscription, tail_span)) => {
                            base.0 = Declarator::ArrayDeclarator {
                                declarator: Box::new((base.0, base.1)),
                                subscription,
                            };
                            base.1 = if has_placeholder_span {
                                tail_span
                            } else {
                                join_spans(base_span, tail_span)
                            };
                        }
                        Err((param_list, tail_span)) => {
                            base.0 = Declarator::FunctionDeclarator {
                                declarator: Box::new((base.0, base.1)),
                                param_list,
                            };
                            base.1 = if has_placeholder_span {
                                tail_span
                            } else {
                                join_spans(base_span, tail_span)
                            };
                        }
                    }
                }
                base
            });

        just(Token::Star)
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
            })
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
                .then_ignore(just(Token::Ellipsis))
                .then(expression(assignment_expression(
                    resolver.clone(),
                    stmt_rec.clone(),
                )))
                .delimited_by(just(Token::LBracket), just(Token::RBracket))
                .map(|(start, end)| Designator::Range(start, end)),
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

            let Ok(decl) = inp.parse(
                declarator(
                    current_resolver.clone(),
                    assignment_expression(current_resolver.clone(), stmt_rec.clone()),
                )
                .filter(|d| declarator_has_name(&d.0))
                .filter(|d| function_decl_direct_inner_is_not_function(&d.0)),
            ) else {
                inp.rewind(declarator_checkpoint);
                break;
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
            let is_transparent_union = inp.parse(just(Token::TransparentUnionAttr)).is_ok();

            let item_span = inp.span_since(&item_start);
            result.push((
                InitDeclarator {
                    declarator: decl,
                    initializer: init,
                    is_transparent_union,
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
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => declarator_has_name(&declarator.0),
    }
}

// In C, a function cannot return a function (only a pointer to one). A valid
// function-definition declarator therefore never has a FunctionDeclarator
// immediately wrapping another FunctionDeclarator. When a typedef name such as
// `int8_t` is also a valid identifier token, the declarator parser can greedily
// misparse `static int8_t (fn_name)(params) { ... }` as a function named
// `int8_t` whose parameter list is `(fn_name)`. Rejecting the doubly-nested
// FunctionDeclarator avoids that ambiguity and forces the left-recursion loop
// to consume `int8_t` as a declaration specifier before trying the declarator.
fn function_decl_direct_inner_is_not_function<R: TypeResolver>(decl: &Declarator<R>) -> bool {
    match decl {
        Declarator::FunctionDeclarator { declarator, .. } => {
            !matches!(&declarator.0, Declarator::FunctionDeclarator { .. })
        }
        _ => true,
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
            .filter(|decl| function_decl_direct_inner_is_not_function(&decl.0))
            .then_ignore(look_ahead(Token::LBrace))
            .then(lazy_compound_statement()),
        declaration_specifier(declarator.clone(), expr.clone(), resolver.clone()),
    )
    .map(
        |(declaration_specifiers, (declarator, body))| Declaration::FunctionDefinition {
            attrs: Vec::new(),
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
            attrs: Vec::new(),
            declaration_specifiers,
            declarators,
        },
    );

    let parser = if resolver.rust_style_syntax_enabled() {
        choice((
            rust_style_function_definition(resolver.clone()),
            rust_style_type_definition(resolver.clone()),
            simple,
            function,
        ))
        .boxed()
    } else {
        choice((simple, function)).boxed()
    };

    parser.map_with(move |v, e| {
        let nr = resolver.register_decl(&v);
        ((v, e.span()), nr)
    })
}

fn use_tree<'src, I>() -> impl Parser<
    'src,
    I,
    Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>,
    extra::Err<Rich<'src, Token, Span>>,
> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|tree| {
        let path = identifier()
            .separated_by(just(Token::ColonColon))
            .at_least(1)
            .collect::<Vec<_>>();

        let alias = just(Token::Ident("as".to_string()))
            .ignore_then(identifier())
            .or_not();

        let star =
            just(Token::Star).map_with(|_, e| vec![(vec![("*".to_string(), e.span())], None)]);

        let group_or_simple = path
            .then(
                just(Token::ColonColon)
                    .ignore_then(choice((
                        tree.clone()
                            .separated_by(just(Token::Comma))
                            .allow_trailing()
                            .collect::<Vec<Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>>>()
                            .delimited_by(just(Token::LBrace), just(Token::RBrace))
                            .map(|nested_lists| {
                                nested_lists.into_iter().flatten().collect::<Vec<_>>()
                            }),
                        just(Token::Star)
                            .map_with(|_, e| vec![(vec![("*".to_string(), e.span())], None)]),
                    )))
                    .or_not(),
            )
            .then(alias)
            .map(|((prefix, nested), alias)| {
                if let Some(nested_items) = nested {
                    let mut flattened = Vec::new();
                    for (mut nested_path, nested_alias) in nested_items {
                        let mut full_path = prefix.clone();
                        full_path.append(&mut nested_path);
                        flattened.push((full_path, nested_alias));
                    }
                    flattened
                } else {
                    vec![(prefix, alias)]
                }
            });

        let just_group = tree
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<Vec<(Vec<Spanned<String>>, Option<Spanned<String>>)>>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(|nested_lists| nested_lists.into_iter().flatten().collect());

        choice((group_or_simple, just_group, star))
    })
}

fn use_item<'src, I>()
-> impl Parser<'src, I, Vec<Spanned<UseItem>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    just(Token::Ident("use".to_owned()))
        .ignore_then(just(Token::ColonColon).or_not())
        .ignore_then(use_tree())
        .then_ignore(just(Token::Semicolon))
        .map_with(|items, e| {
            items
                .into_iter()
                .map(|(path, alias)| {
                    (
                        UseItem {
                            attrs: Vec::new(),
                            path,
                            alias,
                        },
                        e.span(),
                    )
                })
                .collect()
        })
}

fn use_item_with_attrs<'src, I>(
    attrs: Vec<Spanned<RustAttribute>>,
) -> impl Parser<'src, I, Vec<Spanned<UseItem>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    just(Token::Ident("use".to_owned()))
        .ignore_then(just(Token::ColonColon).or_not())
        .ignore_then(use_tree())
        .then_ignore(just(Token::Semicolon))
        .map_with(move |items, e| {
            items
                .into_iter()
                .map(|(path, alias)| {
                    (
                        UseItem {
                            attrs: attrs.clone(),
                            path,
                            alias,
                        },
                        e.span(),
                    )
                })
                .collect()
        })
}

fn inline_mod_body<'src, I>()
-> impl Parser<'src, I, Spanned<Vec<Spanned<Token>>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|block| {
        let content = choice((
            any()
                .filter(|t| !matches!(t, Token::LBrace | Token::RBrace))
                .ignored(),
            block.ignored(),
        ))
        .repeated()
        .ignored();

        content.delimited_by(just(Token::LBrace), just(Token::RBrace))
    })
    .map_with(|(), e| {
        let slice: &[Spanned<Token>] = e.slice();
        // slice includes the surrounding { }, strip them to get inner tokens
        let inner: Vec<Spanned<Token>> = if slice.len() >= 2 {
            slice[1..slice.len() - 1].to_vec()
        } else {
            Vec::new()
        };
        let span = slice_span(slice, e.span());
        (inner, span)
    })
}

fn mod_item<'src, I>()
-> impl Parser<'src, I, Spanned<ModItem>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let mod_kw = just(Token::Ident("mod".to_owned()));

    let file_mod = mod_kw
        .clone()
        .ignore_then(identifier())
        .then_ignore(just(Token::Semicolon))
        .map(|name| ModItem {
            attrs: Vec::new(),
            name,
            inline_content: None,
        });

    let inline_mod = mod_kw
        .ignore_then(identifier())
        .then(inline_mod_body())
        .map(|(name, content)| ModItem {
            attrs: Vec::new(),
            name,
            inline_content: Some(content),
        });

    choice((file_mod, inline_mod)).map_with(|r, e| (r, e.span()))
}

fn mod_item_with_attrs<'src, I>(
    attrs: Vec<Spanned<RustAttribute>>,
) -> impl Parser<'src, I, Spanned<ModItem>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let mod_kw = just(Token::Ident("mod".to_owned()));

    let attrs_for_file = attrs.clone();
    let file_mod = mod_kw
        .clone()
        .ignore_then(identifier())
        .then_ignore(just(Token::Semicolon))
        .map(move |name| ModItem {
            attrs: attrs_for_file.clone(),
            name,
            inline_content: None,
        });

    let inline_mod = mod_kw
        .ignore_then(identifier())
        .then(inline_mod_body())
        .map(move |(name, content)| ModItem {
            attrs: attrs.clone(),
            name,
            inline_content: Some(content),
        });

    choice((file_mod, inline_mod)).map_with(|r, e| (r, e.span()))
}

fn pragma_pack_action(ident: &str) -> Option<co2_ast::PackAction> {
    use co2_ast::PackAction;
    if ident == "__ccc_pack_pop" {
        return Some(PackAction::Pop);
    }
    if ident == "__ccc_pack_reset" {
        return Some(PackAction::Reset);
    }
    if ident == "__ccc_pack_push_only" {
        return Some(PackAction::PushOnly);
    }
    if let Some(n_str) = ident.strip_prefix("__ccc_pack_push_")
        && let Ok(n) = n_str.parse::<u32>()
    {
        return Some(PackAction::PushSet(n));
    }
    if let Some(n_str) = ident.strip_prefix("__ccc_pack_set_")
        && let Ok(n) = n_str.parse::<u32>()
    {
        return Some(PackAction::Set(n));
    }
    None
}

fn break_co2_item<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, (Spanned<Declaration<R>>, R), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    just(Token::Break)
        .then(just(Token::Ident("co2".to_string())))
        .then_ignore(just(Token::Semicolon))
        .map_with(move |_token, e| {
            let decl = (Declaration::BreakCo2, e.span());
            let next_resolver = resolver.register_decl(&decl.0);
            (decl, next_resolver)
        })
}

fn pragma_pack_item<'src, I, R: TypeResolver>(
    resolver: R,
) -> impl Parser<'src, I, (Spanned<Declaration<R>>, R), extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    select! {
        Token::Ident(s) if pragma_pack_action(&s).is_some() => pragma_pack_action(&s).unwrap(),
    }
    .then_ignore(just(Token::Semicolon))
    .map_with(move |action, e| {
        let decl = (
            Declaration::PragmaPack {
                action: action.clone(),
            },
            e.span(),
        );
        let next_resolver = resolver.register_decl(&decl.0);
        (decl, next_resolver)
    })
}

fn attach_attrs_to_declaration<R: TypeResolver>(
    mut decl: Declaration<R>,
    attrs: Vec<Spanned<RustAttribute>>,
) -> Declaration<R> {
    match &mut decl {
        Declaration::FunctionDefinition {
            attrs: decl_attrs, ..
        }
        | Declaration::Declaration {
            attrs: decl_attrs, ..
        }
        | Declaration::RustTypeAlias {
            attrs: decl_attrs, ..
        }
        | Declaration::RustStruct {
            attrs: decl_attrs, ..
        } => *decl_attrs = attrs,
        Declaration::PragmaPack { .. } | Declaration::BreakCo2 => {}
    }
    decl
}

fn attrs_are_outer(attrs: &[Spanned<RustAttribute>]) -> bool {
    attrs.iter().all(|(attr, _)| !attr.is_inner())
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
        Mod(Spanned<ModItem>),
        Declaration(Spanned<Declaration<R>>),
        Empty,
    }

    custom(move |inp| {
        let mut current_resolver = resolver.clone();
        let mut items = Vec::new();
        let mut tu_attrs = Vec::new();

        loop {
            let checkpoint = inp.save();
            if inp.next().is_none() {
                inp.rewind(checkpoint);
                break;
            }
            inp.rewind(checkpoint);

            let mut attrs = inp.parse(rust_attrs())?;
            if let Some(first_outer) = attrs.iter().position(|(attr, _)| !attr.is_inner())
                && first_outer > 0
            {
                tu_attrs.extend(attrs.drain(..first_outer));
            }

            let item =
                if !attrs.is_empty() {
                    if attrs.iter().all(|(attr, _)| attr.is_inner()) {
                        tu_attrs.extend(attrs);
                        continue;
                    }
                    if !attrs_are_outer(&attrs) {
                        let attr_span =
                            attrs.first().zip(attrs.last()).map_or(
                                Span::from_parts(FileId::INVALID, 0..0),
                                |(first, last)| join_spans(first.1, last.1),
                            );
                        return Err(Rich::custom(
                            attr_span,
                            "inner doc comments are only supported before module contents",
                        ));
                    }
                    if let Ok(use_items) = inp.parse(use_item_with_attrs(attrs.clone())) {
                        for item in use_items {
                            items.push(TranslationUnitItem::Use(item));
                        }
                        TranslationUnitItem::Empty
                    } else if let Ok(item) = inp.parse(mod_item_with_attrs(attrs.clone())) {
                        TranslationUnitItem::Mod(item)
                    } else if let Ok((decl, next_resolver)) = inp.parse(
                        choice((
                            rust_style_function_definition_with_attrs(
                                current_resolver.clone(),
                                attrs.clone(),
                            ),
                            rust_style_type_definition_with_attrs(
                                current_resolver.clone(),
                                attrs.clone(),
                            ),
                            rust_style_struct_definition_with_attrs(
                                current_resolver.clone(),
                                attrs.clone(),
                            ),
                        ))
                        .map_with(|v, e| {
                            let nr = current_resolver.register_decl(&v);
                            ((v, e.span()), nr)
                        }),
                    ) {
                        current_resolver = next_resolver;
                        TranslationUnitItem::Declaration(decl)
                    } else if let Ok((decl, next_resolver)) = inp.parse(declaration(
                        current_resolver.clone(),
                        statement(current_resolver.clone()),
                    )) {
                        let decl = (attach_attrs_to_declaration(decl.0, attrs.clone()), decl.1);
                        current_resolver = next_resolver;
                        TranslationUnitItem::Declaration(decl)
                    } else {
                        let attr_span =
                            attrs.first().zip(attrs.last()).map_or(
                                Span::from_parts(FileId::INVALID, 0..0),
                                |(first, last)| join_spans(first.1, last.1),
                            );
                        return Err(Rich::custom(
                            attr_span,
                            "attributes are only supported on rust items",
                        ));
                    }
                } else if let Ok(use_items) = inp.parse(use_item()) {
                    for item in use_items {
                        items.push(TranslationUnitItem::Use(item));
                    }
                    TranslationUnitItem::Empty
                } else if let Ok(item) = inp.parse(mod_item()) {
                    TranslationUnitItem::Mod(item)
                } else if let Ok((decl, next_resolver)) =
                    inp.parse(break_co2_item(current_resolver.clone()))
                {
                    current_resolver = next_resolver;
                    TranslationUnitItem::Declaration(decl)
                } else if let Ok((decl, next_resolver)) =
                    inp.parse(pragma_pack_item(current_resolver.clone()))
                {
                    current_resolver = next_resolver;
                    TranslationUnitItem::Declaration(decl)
                } else if let Ok((decl, next_resolver)) = inp.parse(choice((
                    rust_style_struct_definition(current_resolver.clone()).map_with(|v, e| {
                        let nr = current_resolver.register_decl(&v);
                        ((v, e.span()), nr)
                    }),
                    declaration(
                        current_resolver.clone(),
                        statement(current_resolver.clone()),
                    ),
                ))) {
                    current_resolver = next_resolver;
                    TranslationUnitItem::Declaration(decl)
                } else {
                    inp.parse(just(Token::Semicolon))?;
                    TranslationUnitItem::Empty
                };
            items.push(item);
        }

        Ok((items, tu_attrs))
    })
    .map_with(|(items, tu_attrs), e| {
        let mut rust_use_items = Vec::new();
        let mut rust_mod_items = Vec::new();
        let mut declarations = Vec::new();
        for item in items {
            match item {
                TranslationUnitItem::Use(item) => rust_use_items.push(item),
                TranslationUnitItem::Mod(item) => rust_mod_items.push(item),
                TranslationUnitItem::Declaration(item) => declarations.push(item),
                TranslationUnitItem::Empty => {}
            }
        }
        (
            TranslationUnit {
                attrs: tu_attrs,
                rust_use_items,
                rust_mod_items,
                items: declarations,
            },
            e.span(),
        )
    })
}
