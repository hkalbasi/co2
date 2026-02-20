use std::fmt::Display;

use crate::{Span, Spanned, lexer::Token};
use chumsky::{
    input::{SliceInput, ValueInput},
    prelude::*,
};
use itertools::Itertools as _;

#[derive(Debug, Clone)]
pub struct LazyCompoundStatement {
    pub tokens: Spanned<Vec<Spanned<Token>>>,
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
            _ => Err(Rich::custom(
                inp.span_since(&before),
                "failed to look ahead",
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

#[derive(Debug, Clone)]
pub struct CompoundStatement {
    pub statements: Vec<Spanned<StatementOrDeclaration>>,
}

pub fn compound_statement<'src, I>()
-> impl Parser<'src, I, Spanned<CompoundStatement>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    statement_or_declaration()
        .repeated()
        .collect()
        .map(|statements| CompoundStatement { statements })
        .map_with(|r, e| (r, e.span()))
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
}

#[derive(Debug, Clone)]
pub enum StatementOrDeclaration {
    Declaration(Spanned<Declaration>),
    Statement(Spanned<Statement>),
}

fn statement_or_declaration<'src, I>()
-> impl Parser<'src, I, Spanned<StatementOrDeclaration>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    declaration()
        .map(StatementOrDeclaration::Declaration)
        .or(statement().map(StatementOrDeclaration::Statement))
        .map_with(|r, e| (r, e.span()))
}

#[derive(Debug, Clone)]
pub enum Statement {
    Return(Option<Spanned<Expression>>),
    Expression(Spanned<Expression>),
}

fn statement<'src, I>()
-> impl Parser<'src, I, Spanned<Statement>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let jump_statement = just(Token::Return)
        .ignore_then(expression().or_not())
        .map(|exp| Statement::Return(exp))
        .then_ignore(just(Token::Semicolon));

    let expression_statement = expression()
        .map(Statement::Expression)
        .then_ignore(just(Token::Semicolon));

    choice((jump_statement, expression_statement)).map_with(|r, e| (r, e.span()))
}

#[derive(Debug, Clone)]
pub enum Constant {
    Int(i32),
    String(String),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Empty,
    Constant(Constant),
    Identifier(Spanned<RustPath>),
    Field(Box<Spanned<Expression>>, Spanned<String>),
    InitList(Vec<Spanned<Expression>>),
    Subscript(Box<Spanned<Expression>>, Box<Spanned<Expression>>),
    Call {
        func: Box<Spanned<Expression>>,
        params: Vec<Spanned<Expression>>,
    },
    BinOp(Box<Spanned<Expression>>, BinOp, Box<Spanned<Expression>>),
}

impl Expression {
    fn dummy() -> Box<Spanned<Expression>> {
        Box::new((Expression::Empty, SimpleSpan::from(0..0)))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
}

fn expression<'src, I>()
-> impl Parser<'src, I, Spanned<Expression>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let primary_expression = choice((
            rust_path().map(Expression::Identifier),
            select! {
                Token::Integer(i, _) => Expression::Constant(Constant::Int(i.parse().unwrap())),
                Token::FloatLit(i, _) => Expression::Constant(Constant::Int(i.parse().unwrap())),
                Token::StringLit(s) => Expression::Constant(Constant::String(s)),
            },
            rec.clone()
                .separated_by(just(Token::Comma))
                .collect()
                .delimited_by(just(Token::LBrace), just(Token::RBrace))
                .map(Expression::InitList),
            rec.clone()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .map(|x: (Expression, SimpleSpan)| x.0),
        ))
        .map_with(|r, e| (r, e.span()));

        let postfix_expression = primary_expression
            .then(
                choice((
                    rec.clone()
                        .delimited_by(just(Token::LBracket), just(Token::RBracket))
                        .map(|sub| Expression::Subscript(Expression::dummy(), Box::new(sub))),
                    rec.separated_by(just(Token::Comma))
                        .collect()
                        .map(|params| Expression::Call {
                            func: Expression::dummy(),
                            params,
                        })
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                    just(Token::Dot)
                        .ignore_then(identifier())
                        .map(|ident| Expression::Field(Expression::dummy(), ident)),
                ))
                .repeated()
                .collect::<Vec<_>>(),
            )
            .map(|(mut main, posts)| {
                for mut post in posts {
                    let span = main.1;
                    match &mut post {
                        Expression::Empty
                        | Expression::Identifier(_)
                        | Expression::Constant(_)
                        | Expression::InitList(_)
                        | Expression::BinOp(..) => {
                            unreachable!()
                        }
                        Expression::Subscript(target, _)
                        | Expression::Call {
                            func: target,
                            params: _,
                        }
                        | Expression::Field(target, _) => **target = main,
                    }
                    main = (post, span);
                }
                main
            });

        let mul = just(Token::Star).to(BinOp::Mul);
        let add = just(Token::Plus).to(BinOp::Add);
        let sub = just(Token::Minus).to(BinOp::Sub);

        let multiplicative = postfix_expression
            .clone()
            .then(mul.then(postfix_expression).repeated().collect::<Vec<_>>())
            .map(|(head, tails)| {
                let mut expr = head;
                for (op, rhs) in tails {
                    let span = expr.1;
                    expr = (Expression::BinOp(Box::new(expr), op, Box::new(rhs)), span);
                }
                expr
            });

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
            .map_with(|r, e| (r.0, e.span()));

        additive
    })
}

#[derive(Debug, Clone)]
pub struct LazySubscription {
    #[allow(dead_code)]
    tokens: Vec<Spanned<Token>>,
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

#[derive(Debug, Clone)]
pub enum RustPathSegment {
    Ident(String),
    Generics(Vec<Spanned<RustPath>>),
}

#[derive(Debug, Clone)]
pub struct RustPath {
    pub segments: Vec<Spanned<RustPathSegment>>,
}

impl RustPath {
    pub fn from_ident((ident, span): Spanned<String>) -> RustPath {
        RustPath {
            segments: vec![(RustPathSegment::Ident(ident), span)],
        }
    }

    pub fn to_pretty(&self) -> String {
        self.segments
            .iter()
            .map(|seg| match &seg.0 {
                RustPathSegment::Ident(s) => s.clone(),
                RustPathSegment::Generics(parts) => {
                    let inner = parts
                        .iter()
                        .map(|p| p.0.to_pretty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("<{inner}>")
                }
            })
            .collect::<Vec<_>>()
            .join("::")
    }
}

impl Display for RustPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            &self
                .segments
                .iter()
                .map(|x| match &x.0 {
                    RustPathSegment::Ident(ident) => ident.clone(),
                    RustPathSegment::Generics(rust_paths) => {
                        format!(
                            "<{}>",
                            rust_paths.iter().map(|x| x.0.to_string()).join(", ")
                        )
                    }
                })
                .join("::"),
        )
    }
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

#[derive(Debug, Clone)]
pub enum TypeSpecifier {
    Int,
    Void,
    Char,
    Short,
    Long,
    Float,
    Double,
    Signed,
    Unsigned,
    StructOrUnion {
        kind: StructOrUnionKind,
        specifier: StructOrUnionSpecifier,
    },
    TypedefName(Spanned<RustPath>),
}

#[derive(Debug, Clone)]
pub struct StructOrUnionField {
    pub specifiers: Vec<Spanned<TypeSpecifier>>,
    pub declarators: Vec<Spanned<StructDeclarator>>,
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

fn struct_or_union_fields<'src, I>(
    type_specifier_rec: impl Parser<
        'src,
        I,
        Spanned<TypeSpecifier>,
        extra::Err<Rich<'src, Token, Span>>,
    > + Clone
    + 'src,
    declarator_rec: impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Vec<Spanned<StructOrUnionField>>, extra::Err<Rich<'src, Token, Span>>> + Clone
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

#[derive(Debug, Clone)]
pub enum StructOrUnionSpecifier {
    Defined {
        ident: Spanned<String>,
        fields: Vec<Spanned<StructOrUnionField>>,
    },
    Declared {
        ident: Spanned<String>,
    },
    Anonymous {
        fields: Vec<Spanned<StructOrUnionField>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum StructOrUnionKind {
    Struct,
    Union,
}

fn type_specifier<'src, I>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<TypeSpecifier>, extra::Err<Rich<'src, Token, Span>>> + Clone
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
            )),
        ))
        .map(|(kind, specifier)| TypeSpecifier::StructOrUnion { kind, specifier });

        choice([
            just(Token::Int).to(TypeSpecifier::Int),
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
        .or(rust_path().map(TypeSpecifier::TypedefName))
        .map_with(|r, e| (r, e.span()))
        .labelled("Type specifier")
    })
}

#[derive(Debug, Clone)]
pub enum StorageClassSpecifier {
    Typedef,
    Extern,
    Static,
    ThreadLocal,
    Auto,
    Register,
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
        // just(Token::ThreadLocal).to(StorageClassSpecifier::ThreadLocal),
        just(Token::Auto).to(StorageClassSpecifier::Auto),
        just(Token::Register).to(StorageClassSpecifier::Register),
    ])
    .labelled("Storage specifier")
    .map_with(|r, e| (r, e.span()))
}

#[derive(Debug, Clone)]
pub enum TypeQualifier {
    Const,
    Restrict,
    Volatile,
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

#[derive(Debug, Clone)]
pub enum DeclarationSpecifier {
    TypeSpecifier(Spanned<TypeSpecifier>),
    TypeQualifier(Spanned<TypeQualifier>),
    StorageSpecifier(Spanned<StorageClassSpecifier>),
}

fn declaration_specifier<'src, I>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, Spanned<DeclarationSpecifier>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    choice((
        type_specifier(declarator_rec).map(DeclarationSpecifier::TypeSpecifier),
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

#[derive(Debug, Clone)]
pub struct ParameterList {
    pub parameters: Vec<(Vec<Spanned<DeclarationSpecifier>>, Spanned<Declarator>)>,
    pub ellipsis: bool,
}

fn parameter_type_list<'src, I>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>>
    + Clone
    + 'src,
) -> impl Parser<'src, I, ParameterList, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let single = left_recursion(
        declarator_rec
            .clone()
            .then_ignore(look_ahead(Token::RParen).or(look_ahead(Token::Comma))),
        declaration_specifier(declarator_rec),
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

#[derive(Debug, Clone)]
pub enum Declarator {
    Abstract,
    Identifier(Spanned<String>),
    FunctionDeclarator {
        declarator: Box<Spanned<Declarator>>,
        param_list: ParameterList,
    },
    PointerDeclarator {
        declarator: Box<Spanned<Declarator>>,
        qualifiers: Vec<Spanned<TypeQualifier>>,
    },
    ArrayDeclarator {
        declarator: Box<Spanned<Declarator>>,
        subscription: Spanned<LazySubscription>,
    },
}

fn declarator<'src, I>()
-> impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    recursive(|rec| {
        let ident = identifier()
            .map(Declarator::Identifier)
            .or(empty().to(Declarator::Abstract))
            .map_with(|r, e| (r, e.span()));

        let param_list = parameter_type_list(rec).map(Err);
        let subscription = lazy_subscription().map(Ok);

        let direct_declarator = ident
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

#[derive(Debug, Clone)]
pub struct StructDeclarator {
    pub declarator: Spanned<Declarator>,
    pub bits: Option<Spanned<String>>,
}

fn struct_declarator<'src, I>(
    declarator_rec: impl Parser<'src, I, Spanned<Declarator>, extra::Err<Rich<'src, Token, Span>>>
    + Clone,
) -> impl Parser<'src, I, Spanned<StructDeclarator>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    declarator_rec
        .then(just(Token::Colon).ignore_then(number()).or_not())
        .map(|(declarator, bits)| StructDeclarator { declarator, bits })
        .map_with(|r, e| (r, e.span()))
}

#[derive(Debug, Clone)]
pub struct InitDeclarator {
    pub declarator: Spanned<Declarator>,
    pub initializer: Option<Spanned<Expression>>,
}

fn init_declarator_list<'src, I>()
-> impl Parser<'src, I, Vec<Spanned<InitDeclarator>>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    declarator()
        .then(just(Token::Assign).ignore_then(expression()).or_not())
        .map(|(declarator, initializer)| InitDeclarator {
            declarator,
            initializer,
        })
        .map_with(|r, e| (r, e.span()))
        .separated_by(just(Token::Comma))
        .collect()
}

#[derive(Debug, Clone)]
pub enum Declaration {
    FunctionDefinition {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarator: Spanned<Declarator>,
        body: Spanned<LazyCompoundStatement>,
    },
    Declaration {
        declaration_specifiers: Vec<Spanned<DeclarationSpecifier>>,
        declarators: Vec<Spanned<InitDeclarator>>,
    },
}

fn declaration<'src, I>()
-> impl Parser<'src, I, Spanned<Declaration>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    let function = left_recursion(
        group((
            identifier(),
            parameter_type_list(declarator()),
            lazy_compound_statement(),
        )),
        declaration_specifier(declarator()),
    )
    .map(
        |(declaration_specifiers, ((ident, ident_span), params, body))| {
            Declaration::FunctionDefinition {
                declaration_specifiers,
                declarator: (
                    Declarator::FunctionDeclarator {
                        declarator: Box::new((
                            Declarator::Identifier((ident, ident_span)),
                            ident_span,
                        )),
                        param_list: params,
                    },
                    ident_span,
                ),
                body,
            }
        },
    );

    let simple = left_recursion(
        init_declarator_list().then_ignore(just(Token::Semicolon)),
        declaration_specifier(declarator()),
    )
    .map(
        |(declaration_specifiers, declarators)| Declaration::Declaration {
            declaration_specifiers,
            declarators,
        },
    );

    choice((function, simple)).map_with(|r, e| (r, e.span()))
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: Vec<Spanned<String>>,
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

#[derive(Debug)]
pub struct TranslationUnit {
    pub rust_use_items: Vec<Spanned<UseItem>>,
    pub items: Vec<Spanned<Declaration>>,
}

pub fn translation_unit<'src, I>()
-> impl Parser<'src, I, Spanned<TranslationUnit>, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>
        + SliceInput<'src, Slice = &'src [Spanned<Token>]>,
{
    use_item()
        .repeated()
        .collect()
        .then(declaration().repeated().collect())
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
