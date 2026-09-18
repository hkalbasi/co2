//! Expression and initializer parsing (Pratt parser, C precedence).

use super::parser::{
    P, PR, merge_string_literals, parse_comma_list, parse_hex_float_constant,
    parse_rust_ty_stateless_inner, rust_path_span,
};
use co2_ast::TypeResolver;
use co2_ast::{
    BinOp, CharPrefix, Constant, Designator, Expression, FloatSuffix, GenericAssociation,
    Initializer, InitializerItem, IntegerSuffix, RustPath, RustPathSegment, Span, Spanned,
    StatelessResolver, Token, TypeQueryResult, UnaryOp, UpdateOp, parse_unsigned_integer_constant,
};

// ── Entry levels ───────────────────────────────────────────────────────

/// `assignment (, assignment)*` — left folded, span pinned to the head.
pub(crate) fn parse_expression<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> PR<Spanned<Expression<R>>> {
    let mut head = parse_assignment(p)?;
    let span = head.1;
    while p.eat(&Token::Comma).is_some() {
        let rhs = parse_assignment(p)?;
        head = (
            Expression::BinOp(Box::new(head), BinOp::Comma, Box::new(rhs)),
            span,
        );
    }
    Ok(head)
}

pub(crate) fn parse_assignment<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> PR<Spanned<Expression<R>>> {
    let start = p.pos;
    let lhs = parse_conditional(p)?;
    let op: Option<Option<BinOp>> = match p.peek(0) {
        Some(Token::Assign) => Some(None),
        Some(Token::PlusAssign) => Some(Some(BinOp::Add)),
        Some(Token::MinusAssign) => Some(Some(BinOp::Sub)),
        Some(Token::StarAssign) => Some(Some(BinOp::Mul)),
        Some(Token::SlashAssign) => Some(Some(BinOp::Div)),
        Some(Token::PercentAssign) => Some(Some(BinOp::Rem)),
        Some(Token::PipeAssign) => Some(Some(BinOp::BitOr)),
        Some(Token::CaretAssign) => Some(Some(BinOp::BitXor)),
        Some(Token::AmpAssign) => Some(Some(BinOp::BitAnd)),
        Some(Token::ShlAssign) => Some(Some(BinOp::Shl)),
        Some(Token::ShrAssign) => Some(Some(BinOp::Shr)),
        _ => None,
    };
    match op {
        // No assignment operator: still re-span to everything consumed,
        // matching the old outer `map_with` on assignment expressions.
        None => Ok((lhs.0, p.span_since(start))),
        Some(op) => {
            p.pos += 1;
            let rhs = parse_assignment(p)?;
            let span = p.span_since(start);
            match op {
                Some(op) => Ok((
                    Expression::AssignWithOp {
                        lhs: Box::new(lhs),
                        op,
                        rhs: Box::new(rhs),
                    },
                    span,
                )),
                None => Ok((
                    Expression::BinOp(Box::new(lhs), BinOp::Assign, Box::new(rhs)),
                    span,
                )),
            }
        }
    }
}

fn parse_conditional<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let cond = parse_lor(p)?;
    if p.eat(&Token::Question).is_none() {
        return Ok(cond);
    }
    let span = cond.1;
    // GNU elvis `a ?: b`: middle may be omitted.
    let then_opt = if p.at(&Token::Colon) {
        None
    } else {
        Some(parse_expression(p)?)
    };
    p.expect(&Token::Colon, ":")?;
    let else_expr = parse_assignment(p)?;
    Ok((
        Expression::Conditional {
            cond: Box::new(cond),
            then_expr: then_opt.map(Box::new),
            else_expr: Box::new(else_expr),
        },
        span,
    ))
}

// Each layer left-folds with the head span (matches old behavior).
fn parse_binop<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
    next: fn(&mut P<'a, R>) -> PR<Spanned<Expression<R>>>,
    op_of: impl Fn(&Token) -> Option<BinOp>,
) -> PR<Spanned<Expression<R>>> {
    let mut head = next(p)?;
    let span = head.1;
    loop {
        let op = match p.peek(0) {
            Some(t) => match op_of(t) {
                Some(op) => op,
                None => break,
            },
            None => break,
        };
        p.pos += 1;
        let rhs = next(p)?;
        head = (Expression::BinOp(Box::new(head), op, Box::new(rhs)), span);
    }
    Ok(head)
}

fn parse_lor<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_land, |t| match t {
        Token::Or => Some(BinOp::Or),
        _ => None,
    })
}

fn parse_land<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_bitor, |t| match t {
        Token::And => Some(BinOp::And),
        _ => None,
    })
}

fn parse_bitor<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_bitxor, |t| match t {
        Token::Pipe => Some(BinOp::BitOr),
        _ => None,
    })
}

fn parse_bitxor<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_bitand, |t| match t {
        Token::Caret => Some(BinOp::BitXor),
        _ => None,
    })
}

fn parse_bitand<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_equality, |t| match t {
        Token::Amp => Some(BinOp::BitAnd),
        _ => None,
    })
}

fn parse_equality<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_relational, |t| match t {
        Token::EqEq => Some(BinOp::Eq),
        Token::Ne => Some(BinOp::Ne),
        _ => None,
    })
}

fn parse_relational<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let mut head = parse_shift(p)?;
    let span = head.1;
    loop {
        let op = match p.peek(0) {
            Some(Token::Lt) => BinOp::Lt,
            Some(Token::Le) => BinOp::Le,
            Some(Token::Gt) => {
                // A `>>` belongs to shift (already consumed below); a lone
                // `>` followed by another `>` here would be a tokenization
                // edge — treat the first as `>` and let the outer level fail
                // loudly rather than silently merging.
                if matches!(p.peek(1), Some(Token::Gt)) {
                    break;
                }
                BinOp::Gt
            }
            Some(Token::Ge) => BinOp::Ge,
            _ => break,
        };
        p.pos += 1;
        let rhs = parse_shift(p)?;
        head = (Expression::BinOp(Box::new(head), op, Box::new(rhs)), span);
    }
    Ok(head)
}

fn parse_shift<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let mut head = parse_additive(p)?;
    let span = head.1;
    loop {
        let op = if p.at(&Token::Shl) {
            p.pos += 1;
            BinOp::Shl
        } else if p.at(&Token::Shr) {
            p.pos += 1;
            BinOp::Shr
        } else if p.at(&Token::Gt) && matches!(p.peek(1), Some(Token::Gt)) {
            p.pos += 2;
            BinOp::Shr
        } else {
            break;
        };
        let rhs = parse_additive(p)?;
        head = (Expression::BinOp(Box::new(head), op, Box::new(rhs)), span);
    }
    Ok(head)
}

fn parse_additive<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_multiplicative, |t| match t {
        Token::Plus => Some(BinOp::Add),
        Token::Minus => Some(BinOp::Sub),
        _ => None,
    })
}

fn parse_multiplicative<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    parse_binop(p, parse_cast, |t| match t {
        Token::Star => Some(BinOp::Mul),
        Token::Slash => Some(BinOp::Div),
        Token::Percent => Some(BinOp::Rem),
        _ => None,
    })
}

// ── Cast / unary / postfix / primary ───────────────────────────────────

fn parse_cast<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    if p.at(&Token::LParen) {
        let cp = p.checkpoint();
        p.pos += 1;
        match p.parse_type_name() {
            Ok(ty) => {
                if p.eat(&Token::RParen).is_some() {
                    match parse_cast(p) {
                        Ok(operand) => {
                            let span = p.span_since(cp.0);
                            return Ok((
                                Expression::Cast {
                                    type_name: Box::new(ty),
                                    expr: Box::new(operand),
                                },
                                span,
                            ));
                        }
                        Err(_) => p.restore(cp),
                    }
                } else {
                    p.restore(cp);
                }
            }
            Err(_) => p.restore(cp),
        }
    }
    parse_unary(p)
}

fn parse_unary<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let start = p.pos;
    match p.peek(0) {
        Some(Token::Sizeof) | Some(Token::Alignof) | Some(Token::Countof) => {
            let kind = p.peek(0).cloned();
            p.pos += 1;
            if p.at(&Token::LParen) {
                let cp = p.checkpoint();
                p.pos += 1;
                match p.parse_type_name() {
                    Ok(ty) => {
                        if p.eat(&Token::RParen).is_some() {
                            let span = p.span_since(start);
                            let ty = Box::new(ty);
                            return Ok((
                                match kind {
                                    Some(Token::Sizeof) => Expression::SizeofType(ty),
                                    Some(Token::Alignof) => Expression::AlignofType(ty),
                                    _ => Expression::CountofType(ty),
                                },
                                span,
                            ));
                        }
                        p.restore(cp);
                    }
                    Err(_) => p.restore(cp),
                }
            }
            let e = parse_unary(p)?;
            let e = Box::new(e);
            Ok((
                match kind {
                    Some(Token::Sizeof) => Expression::Sizeof(e),
                    Some(Token::Alignof) => Expression::Alignof(e),
                    _ => Expression::Countof(e),
                },
                p.span_since(start),
            ))
        }
        Some(Token::Offsetof) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let ty = p.parse_type_name()?;
            p.expect(&Token::Comma, ",")?;
            let (field, field_span) = p.parse_identifier()?;
            let mut designator = vec![co2_ast::OffsetofMember::Field((field, field_span))];
            loop {
                if p.eat(&Token::Dot).is_some() {
                    let (name, span) = p.parse_identifier()?;
                    designator.push(co2_ast::OffsetofMember::Field((name, span)));
                } else if p.at(&Token::LBracket) {
                    p.pos += 1;
                    let idx = parse_expression(p)?;
                    p.expect(&Token::RBracket, "]")?;
                    designator.push(co2_ast::OffsetofMember::Index(Box::new(idx)));
                } else {
                    break;
                }
            }
            p.expect(&Token::RParen, ")")?;
            Ok((
                Expression::Offsetof {
                    ty: Box::new(ty),
                    designator,
                },
                p.span_since(start),
            ))
        }
        Some(Token::BuiltinTypesCompatibleP) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let ty1 = p.parse_type_name()?;
            p.expect(&Token::Comma, ",")?;
            let ty2 = p.parse_type_name()?;
            p.expect(&Token::RParen, ")")?;
            Ok((
                Expression::BuiltinTypesCompatibleP {
                    ty1: Box::new(ty1),
                    ty2: Box::new(ty2),
                },
                p.span_since(start),
            ))
        }
        Some(Token::BuiltinConstantP) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let expr = parse_assignment(p)?;
            p.expect(&Token::RParen, ")")?;
            Ok((
                Expression::BuiltinConstantP {
                    expr: Box::new(expr),
                },
                p.span_since(start),
            ))
        }
        Some(Token::Inc) | Some(Token::Dec) => {
            let op = if matches!(p.peek(0), Some(Token::Inc)) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            p.pos += 1;
            let e = parse_unary(p)?;
            Ok((
                Expression::Update {
                    expr: Box::new(e),
                    op,
                    is_postfix: false,
                },
                p.span_since(start),
            ))
        }
        Some(Token::Bang) | Some(Token::Tilde) | Some(Token::Amp) | Some(Token::Star)
        | Some(Token::Plus) | Some(Token::Minus) => {
            let op = match p.peek(0) {
                Some(Token::Bang) => UnaryOp::Not,
                Some(Token::Tilde) => UnaryOp::Com,
                Some(Token::Amp) => UnaryOp::AddrOf,
                Some(Token::Star) => UnaryOp::Deref,
                Some(Token::Plus) => UnaryOp::Plus,
                _ => UnaryOp::Minus,
            };
            p.pos += 1;
            let e = parse_cast(p)?;
            Ok((Expression::UnaryOp(op, Box::new(e)), p.span_since(start)))
        }
        _ => parse_postfix(p),
    }
}

fn parse_call_args<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Vec<Spanned<Expression<R>>>> {
    parse_comma_list(p, &Token::LParen, &Token::RParen, false, parse_assignment)
}

fn parse_postfix<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let mut node = parse_primary(p)?;
    let span = node.1;
    loop {
        if p.at(&Token::LBracket) {
            p.pos += 1;
            let sub = parse_expression(p)?;
            p.expect(&Token::RBracket, "]")?;
            node = (Expression::Subscript(Box::new(node), Box::new(sub)), span);
        } else if p.at(&Token::LParen) {
            let args = parse_call_args(p)?;
            node = (
                Expression::Call {
                    func: Box::new(node),
                    params: args,
                },
                span,
            );
        } else if p.at(&Token::Dot) || p.at(&Token::Arrow) {
            let is_arrow = p.at(&Token::Arrow);
            p.pos += 1;
            let ident = p.parse_identifier()?;
            if p.at(&Token::ColonColon) {
                p.pos += 1;
                let generics = parse_comma_list(
                    p,
                    &Token::Lt,
                    &Token::Gt,
                    false,
                    parse_rust_ty_stateless_inner,
                )?;
                let params = parse_call_args(p)?;
                node = (
                    Expression::MethodCall {
                        receiver: Box::new(node),
                        method: ident,
                        generics,
                        params,
                    },
                    span,
                );
            } else if is_arrow {
                node = (Expression::Arrow(Box::new(node), ident), span);
            } else {
                node = (Expression::Field(Box::new(node), ident), span);
            }
        } else if p.at(&Token::Inc) || p.at(&Token::Dec) {
            let op = if p.at(&Token::Inc) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            p.pos += 1;
            node = (
                Expression::Update {
                    expr: Box::new(node),
                    op,
                    is_postfix: true,
                },
                span,
            );
        } else {
            break;
        }
    }
    Ok(node)
}

// ── Primary ────────────────────────────────────────────────────────────

fn parse_primary<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Expression<R>>> {
    let start = p.pos;
    let expr = match p.peek(0) {
        Some(Token::VaStart) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let args = parse_assignment(p)?;
            p.expect(&Token::Comma, ",")?;
            let last_param = p.parse_identifier()?;
            p.expect(&Token::RParen, ")")?;
            Expression::VaStart {
                args: Box::new(args),
                last_param,
            }
        }
        Some(Token::VaArg) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let args = parse_assignment(p)?;
            p.expect(&Token::Comma, ",")?;
            let type_name = p.parse_type_name()?;
            p.expect(&Token::RParen, ")")?;
            Expression::VaArg {
                args: Box::new(args),
                type_name,
            }
        }
        Some(Token::VaCopy) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let dest = parse_assignment(p)?;
            p.expect(&Token::Comma, ",")?;
            let src = parse_assignment(p)?;
            p.expect(&Token::RParen, ")")?;
            Expression::VaCopy {
                dest: Box::new(dest),
                src: Box::new(src),
            }
        }
        Some(Token::VaEnd) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let args = parse_assignment(p)?;
            p.expect(&Token::RParen, ")")?;
            Expression::VaEnd {
                args: Box::new(args),
            }
        }
        Some(Token::Generic) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            let controlling = parse_assignment(p)?;
            p.expect(&Token::Comma, ",")?;
            let mut assocs = Vec::new();
            loop {
                let astart = p.pos;
                let assoc = if p.at(&Token::Default) {
                    p.pos += 1;
                    p.expect(&Token::Colon, ":")?;
                    let expr = parse_assignment(p)?;
                    GenericAssociation::Default { expr }
                } else {
                    let type_name = p.parse_type_name()?;
                    p.expect(&Token::Colon, ":")?;
                    let expr = parse_assignment(p)?;
                    GenericAssociation::Type { type_name, expr }
                };
                assocs.push((assoc, p.span_since(astart)));
                if p.eat(&Token::Comma).is_none() {
                    break;
                }
            }
            p.expect(&Token::RParen, ")")?;
            Expression::GenericSelection {
                controlling: Box::new(controlling),
                associations: assocs,
            }
        }
        Some(Token::BuiltinInf) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            p.expect(&Token::RParen, ")")?;
            Expression::Constant(Constant::Float(f64::INFINITY, FloatSuffix::None))
        }
        Some(Token::BuiltinNan) => {
            p.pos += 1;
            p.expect(&Token::LParen, "(")?;
            while matches!(p.peek(0), Some(Token::StringLit(_))) {
                p.pos += 1;
            }
            p.expect(&Token::RParen, ")")?;
            Expression::Constant(Constant::Float(f64::NAN, FloatSuffix::None))
        }
        Some(Token::And) => {
            // `&&ident` — GNU label address. (`&` is Amp; `And` is `&&`.)
            p.pos += 1;
            Expression::LabelAddress(p.parse_identifier()?)
        }
        Some(Token::LParen) if matches!(p.peek(1), Some(Token::LBrace)) => {
            p.pos += 1; // `(`
            let body = p.parse_compound_inner()?;
            p.expect(&Token::RParen, ")")?;
            Expression::GnuStatementExpr {
                body: Box::new(body),
            }
        }
        Some(Token::LParen) => {
            // Compound literal `(T){...}` preferred over `(expr)`.
            let cp = p.checkpoint();
            p.pos += 1;
            let compound = match p.parse_type_name() {
                Ok(ty) => {
                    if p.eat(&Token::RParen).is_some() && p.at(&Token::LBrace) {
                        match parse_init_list(p) {
                            Ok(init) => Some((ty, init)),
                            Err(_) => None,
                        }
                    } else {
                        None
                    }
                }
                Err(_) => None,
            };
            if let Some((ty, init)) = compound {
                Expression::CompoundLiteral {
                    type_name: Box::new(ty),
                    initializer: Box::new(init),
                }
            } else {
                p.restore(cp);
                p.pos += 1; // `(` again
                let inner = parse_expression(p)?;
                p.expect(&Token::RParen, ")")?;
                // Span covers the parentheses (matches old `map_with` span).
                let span = p.span_since(start);
                return Ok((inner.0, span));
            }
        }
        Some(Token::Lt) => return parse_ufcs_path(p, start),
        Some(Token::Ident(_)) | Some(Token::ColonColon) => {
            return parse_normal_path(p, start);
        }
        Some(Token::StringLit(_)) => {
            let mut parts = Vec::new();
            while let Some(Token::StringLit(s)) = p.peek(0) {
                parts.push(s.clone());
                p.pos += 1;
            }
            let span = p.span_since(start);
            Expression::Constant(Constant::String(merge_string_literals(parts, span)))
        }
        Some(Token::Integer(_, _)) | Some(Token::FloatLit(_, _)) | Some(Token::CharLit(_, _)) => {
            return parse_literal(p, start);
        }
        Some(Token::BoolLit(v)) => {
            let v = *v;
            p.pos += 1;
            Expression::Constant(Constant::Bool(v))
        }
        _ => {
            return Err(p.fail_here(format!("expected expression, found {}", p.describe())));
        }
    };
    Ok((expr, p.span_since(start)))
}

/// Classify `path` as an expression; report failures at the start token.
fn resolve_expr_path<'a, R: TypeResolver>(
    p: &P<'a, R>,
    path: &RustPath<StatelessResolver>,
    err_span: Span,
) -> PR<R::ResolvedRustPath> {
    match p.resolver.classify_path(path) {
        Ok((TypeQueryResult::Unsure | TypeQueryResult::Expr, resolved)) => Ok(resolved),
        Ok((TypeQueryResult::Type, _)) => {
            Err(p.fail_at(err_span, "expected expression, found type name".to_string()))
        }
        Err((msg, _)) => Err(p.fail_at(err_span, msg)),
    }
}

/// `<T as Trait>::method` path.
fn parse_ufcs_path<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
    start: usize,
) -> PR<Spanned<Expression<R>>> {
    let err_span = p.cur_span();
    let cp = p.checkpoint();
    p.pos += 1; // `<`
    let result = (|| -> PR<Spanned<Expression<R>>> {
        let (type_path, _) = p.parse_rust_path(true)?;
        if p.eat_ident("as").is_none() {
            return Err(p.fail_here(format!("expected as, found {}", p.describe())));
        }
        let (trait_path, _) = p.parse_rust_path(true)?;
        p.expect(&Token::Gt, ">")?;
        p.expect(&Token::ColonColon, "::")?;
        let method = p.parse_identifier()?;
        let type_segments = type_path.segments;
        let trait_segments = trait_path.segments;
        let qual_span = p.span_since(start);
        let method_span = method.1;
        let qual_segment = (
            RustPathSegment::<StatelessResolver>::Qualified {
                type_segments,
                trait_segments,
            },
            qual_span,
        );
        let path = RustPath {
            segments: vec![
                qual_segment,
                (RustPathSegment::Ident(method.0), method_span),
            ],
        };
        let path_span = rust_path_span(&path, qual_span);
        let resolved = resolve_expr_path(p, &path, err_span)?;
        Ok((
            Expression::Identifier((resolved, path_span)),
            p.span_since(start),
        ))
    })();
    match result {
        Ok(v) => Ok(v),
        Err(_) => {
            p.restore(cp);
            Err(p.fail_here(format!("expected expression, found {}", p.describe())))
        }
    }
}

/// Plain `a::b::c` path with the turbofish-miss check.
fn parse_normal_path<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
    start: usize,
) -> PR<Spanned<Expression<R>>> {
    // See parse_ufcs_path: report classify failures at the start token.
    let err_span = p.cur_span();
    let (path, _) = p.parse_rust_path(false)?;
    let span = p.span_since(start);
    let path_span = rust_path_span(&path, span);
    let has_lt = p.at(&Token::Lt);
    let lt_span = p.cur_span();
    if matches!(
        p.resolver.classify_path(&path),
        Ok((TypeQueryResult::Type, _))
    ) && has_lt
    {
        let ctx = path_span.data().context;
        let s = path_span.data().start;
        let e = lt_span.data().end;
        let span = Span::from_parts(ctx, s..e);
        co2_ast::emit_errors_and_terminate(vec![
            co2_ast::Rich::custom(
                span,
                format!("generic arguments require turbofish syntax: `{path}::<...>`"),
            )
            .map_token(|tok: Token| tok.to_string()),
        ]);
    }
    let resolved = resolve_expr_path(p, &path, err_span)?;
    Ok((Expression::Identifier((resolved, path_span)), span))
}

fn parse_literal<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
    start: usize,
) -> PR<Spanned<Expression<R>>> {
    match p.peek(0) {
        Some(Token::Integer(s, suffix)) => {
            let i = s.clone();
            let suffix = suffix.clone();
            p.pos += 1;
            let span = p.span_since(start);
            let expr = match parse_unsigned_integer_constant(&i) {
                Some(v) => {
                    let suffix = if suffix == IntegerSuffix::None
                        && !i.starts_with("0x")
                        && !i.starts_with("0X")
                        && !i.starts_with("0b")
                        && !i.starts_with("0B")
                        && !(i.len() > 1 && i.starts_with('0'))
                    {
                        IntegerSuffix::NoneDecimal
                    } else {
                        suffix
                    };
                    Expression::Constant(Constant::Int(v as i128, suffix))
                }
                None => {
                    let msg = if i.starts_with("0x") || i.starts_with("0X") {
                        "Invalid hexadecimal int literal"
                    } else if i.starts_with("0b") || i.starts_with("0B") {
                        "Invalid binary int literal"
                    } else {
                        "Invalid integer literal"
                    };
                    co2_ast::emit_errors(vec![co2_ast::Rich::custom(span, msg)]);
                    Expression::Constant(Constant::Int(0, suffix))
                }
            };
            Ok((expr, span))
        }
        Some(Token::FloatLit(s, suffix)) => {
            let i = s.clone();
            let suffix = suffix.clone();
            p.pos += 1;
            let span = p.span_since(start);
            let value = i
                .parse::<f64>()
                .ok()
                .or_else(|| parse_hex_float_constant(&i));
            let expr = match value {
                Some(v) => Expression::Constant(Constant::Float(v, suffix)),
                None => {
                    co2_ast::emit_errors(vec![co2_ast::Rich::custom(
                        span,
                        "Invalid float literal",
                    )]);
                    Expression::Constant(Constant::Float(0.0, suffix))
                }
            };
            Ok((expr, span))
        }
        Some(Token::CharLit(s, prefix)) => {
            let s = s.clone();
            let prefix = *prefix;
            p.pos += 1;
            let span = p.span_since(start);
            let expr = if s.is_empty() {
                co2_ast::emit_errors(vec![co2_ast::Rich::custom(
                    span,
                    "Invalid character constant",
                )]);
                Expression::Constant(Constant::Char(0, prefix))
            } else if prefix == CharPrefix::None && s.len() > 1 {
                let used = if s.len() > 4 {
                    &s[s.len() - 4..]
                } else {
                    &s[..]
                };
                let mut value: u32 = 0;
                for &b in used {
                    value = (value << 8) | u32::from(b);
                }
                Expression::Constant(Constant::Int(i128::from(value as i32), IntegerSuffix::None))
            } else if prefix == CharPrefix::Utf8 {
                let value = if s.len() == 1 {
                    u32::from(s[0])
                } else {
                    String::from_utf8_lossy(&s)
                        .chars()
                        .next()
                        .map(|c| c as u32)
                        .unwrap_or(0)
                };
                Expression::Constant(Constant::Char(value, prefix))
            } else if prefix == CharPrefix::None {
                Expression::Constant(Constant::Char((s[0] as i8) as u32, prefix))
            } else {
                let mut bytes = [0u8; 4];
                for (i, &b) in s.iter().take(4).enumerate() {
                    bytes[i] = b;
                }
                Expression::Constant(Constant::Char(u32::from_le_bytes(bytes), prefix))
            };
            Ok((expr, span))
        }
        _ => Err(p.fail_here(format!("expected literal, found {}", p.describe()))),
    }
}

// ── Initializers ───────────────────────────────────────────────────────

pub(crate) fn parse_initializer<'a, R: TypeResolver>(
    p: &mut P<'a, R>,
) -> PR<Spanned<Initializer<R>>> {
    let start = p.pos;
    if p.at(&Token::LBrace) {
        parse_init_list(p)
    } else {
        let e = parse_assignment(p)?;
        Ok((Initializer::Expr(e), p.span_since(start)))
    }
}

fn parse_init_list<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<Initializer<R>>> {
    let start = p.pos;
    let items = parse_comma_list(p, &Token::LBrace, &Token::RBrace, true, parse_init_item)?;
    Ok((Initializer::List(items), p.span_since(start)))
}

fn parse_init_item<'a, R: TypeResolver>(p: &mut P<'a, R>) -> PR<Spanned<InitializerItem<R>>> {
    let start = p.pos;
    // Try `designators =` first; fall back to a plain initializer.
    let cp = p.checkpoint();
    let mut designators = Vec::new();
    let has_designated = loop {
        match try_parse_designator(p) {
            Some(d) => designators.push(d),
            None => break !designators.is_empty() && p.at(&Token::Assign),
        }
    };
    let init = if has_designated {
        p.pos += 1; // `=`
        parse_initializer(p)?
    } else {
        p.restore(cp);
        parse_initializer(p)?
    };
    Ok((
        InitializerItem {
            designators: if has_designated {
                Some(designators)
            } else {
                None
            },
            initializer: init,
        },
        p.span_since(start),
    ))
}

/// Parse one designator, or return None without consuming on failure.
fn try_parse_designator<'a, R: TypeResolver>(p: &mut P<'a, R>) -> Option<Spanned<Designator<R>>> {
    // `.field`
    if p.at(&Token::Dot) {
        let cp = p.checkpoint();
        p.pos += 1;
        match p.parse_identifier() {
            Ok(name) => {
                let span = p.span_since(cp.0);
                return Some((Designator::Field(name), span));
            }
            Err(_) => {
                p.restore(cp);
                return None;
            }
        }
    }
    // `[expr]` or `[expr ... expr]`
    if p.at(&Token::LBracket) {
        let cp = p.checkpoint();
        p.pos += 1;
        let start_expr = match parse_expression(p) {
            Ok(e) => e,
            Err(_) => {
                p.restore(cp);
                return None;
            }
        };
        if p.eat(&Token::Ellipsis).is_some() {
            match parse_expression(p) {
                Ok(end_expr) => {
                    if p.eat(&Token::RBracket).is_none() {
                        p.restore(cp);
                        return None;
                    }
                    let span = p.span_since(cp.0);
                    return Some((Designator::Range(start_expr, end_expr), span));
                }
                Err(_) => {
                    p.restore(cp);
                    return None;
                }
            }
        }
        if p.eat(&Token::RBracket).is_none() {
            p.restore(cp);
            return None;
        }
        let span = p.span_since(cp.0);
        return Some((Designator::Subscript(start_expr), span));
    }
    None
}
