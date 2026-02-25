use chumsky::span::SimpleSpan;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::{Parser as _, input::Input as _};

use crate::{
    diagnostic::take_errors,
    lexer::lexer,
    parser::{TranslationUnit, translation_unit},
};

mod diagnostic;
mod exp;
mod lexer;
mod parser;

pub use lexer::{FloatSuffix, IntegerSuffix, Token};
pub use parser::{
    BinOp, CompoundStatement, Constant, Declaration, DeclarationSpecifier, Declarator, Designator,
    EnumSpecifier, Enumerator, Expression, InitDeclarator, Initializer, InitializerItem,
    LazyCompoundStatement, RustPath, RustPathSegment, SpecifierQualifier, Statement,
    StatementOrDeclaration, StorageClassSpecifier, StructDeclarator, StructOrUnionField,
    StructOrUnionSpecifier, TranslationUnit as ParsedTranslationUnit, TypeName, TypeQueryResult,
    TypeSpecifier, UnaryOp, UpdateOp, UseItem,
};

// Type definitions
pub type Span = SimpleSpan<usize>;
pub type Spanned<T> = (T, Span);

pub trait TypeResolver {
    fn classify_path(&self, path: &RustPath) -> TypeQueryResult;
}

pub struct AllowAllTypes;

impl TypeResolver for AllowAllTypes {
    fn classify_path(&self, _path: &RustPath) -> TypeQueryResult {
        TypeQueryResult::Type
    }
}

pub fn parse_translation_unit(
    filename: String,
    src: &'static str,
    resolver: &dyn TypeResolver,
) -> Option<Spanned<TranslationUnit>> {
    let (tokens, errs) = lexer().parse(src).into_output_errors();

    if let Some(tokens) = tokens {
        let tokens = tokens.leak();
        let (ast, parse_errs) = translation_unit(resolver)
            .map_with(|ast, e| (ast, e.span()))
            .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
            .into_output_errors();

        if parse_errs.is_empty() {
            if let Some(ast) = ast {
                return Some(ast.0);
            }
        } else {
            for err in parse_errs {
                let e = err.map_token(|tok| tok.to_string());
                Report::build(ReportKind::Error, (filename.clone(), e.span().into_range()))
                    .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                    .with_message(e.to_string())
                    .with_label(
                        Label::new((filename.clone(), e.span().into_range()))
                            .with_message(e.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .with_labels(e.contexts().map(|(label, span)| {
                        Label::new((filename.clone(), span.into_range()))
                            .with_message(format!("while parsing this {label}"))
                            .with_color(Color::Yellow)
                    }))
                    .finish()
                    .print(sources([(filename.clone(), src.to_owned())]))
                    .unwrap();
            }
            std::process::exit(5);
        }
    }

    print_errors_and_terminate(filename, src, errs);
}

pub fn parse_items(filename: String, src: &'static str) -> Option<Spanned<TranslationUnit>> {
    parse_translation_unit(filename, src, &AllowAllTypes)
}

pub fn parse_compound_statement(
    tokens: &[Spanned<Token>],
    filename: String,
    src: &'static str,
    resolver: &dyn TypeResolver,
) -> Option<Spanned<CompoundStatement>> {
    let (ast, parse_errs) = parser::compound_statement(resolver, parser::statement(resolver))
        .parse(tokens.map((src.len()..src.len()).into(), |(t, s)| (t, s)))
        .into_output_errors();

    if parse_errs.is_empty() {
        if let Some(ast) = ast {
            return Some(ast);
        }
    } else {
        for err in parse_errs {
            let e = err.map_token(|tok| tok.to_string());
            Report::build(ReportKind::Error, (filename.clone(), e.span().into_range()))
                .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), e.span().into_range()))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), span.into_range()))
                        .with_message(format!("while parsing this {label}"))
                        .with_color(Color::Yellow)
                }))
                .finish()
                .print(sources([(filename.clone(), src.to_owned())]))
                .unwrap();
        }
        std::process::exit(5);
    }

    print_errors_and_terminate(filename, src, Vec::new());
}

pub fn print_errors_and_terminate(
    filename: String,
    src: &'static str,
    errs: Vec<chumsky::prelude::Rich<'_, char>>,
) -> ! {
    errs.into_iter()
        .map(|e| e.map_token(|c| c.to_string()))
        .chain(
            take_errors()
                .into_iter()
                .map(|e| e.map_token(|tok| tok.to_string())),
        )
        .for_each(|e| {
            Report::build(ReportKind::Error, (filename.clone(), e.span().into_range()))
                .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), e.span().into_range()))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), span.into_range()))
                        .with_message(format!("while parsing this {label}"))
                        .with_color(Color::Yellow)
                }))
                .finish()
                .print(sources([(filename.clone(), src.to_owned())]))
                .unwrap()
        });
    std::process::exit(5);
}

#[cfg(test)]
mod tests {
    use crate::{Declaration, DeclarationSpecifier, TypeSpecifier, parse_items};

    #[test]
    fn array_subscription_constant_len() {
        let src: &'static str = "int arr[2];";
        let parsed = parse_items("test.c".to_owned(), src).expect("failed to parse");
        let (decl, _) = &parsed.0.items[0];
        let Declaration::Declaration {
            declaration_specifiers,
            declarators,
        } = decl
        else {
            panic!("expected declaration");
        };
        let has_int = declaration_specifiers.iter().any(|(spec, _)| {
            matches!(
                spec,
                DeclarationSpecifier::TypeSpecifier((TypeSpecifier::Int, _))
            )
        });
        assert!(has_int);
        let init = &declarators[0].0;
        let crate::Declarator::ArrayDeclarator { subscription, .. } = &init.declarator.0 else {
            panic!("expected array declarator");
        };
        assert_eq!(
            subscription.0.constant_len(),
            Some(2),
            "subscription={subscription:?}"
        );
    }

    #[test]
    fn anonymous_struct_same_field_set_has_same_key() {
        let src_a: &'static str = "struct { int x; int y; };";
        let src_b: &'static str = "struct { int y; int x; };";
        let parsed_a = parse_items("a.c".to_owned(), src_a).expect("failed to parse a");
        let parsed_b = parse_items("b.c".to_owned(), src_b).expect("failed to parse b");

        let key_of = |decl: &Declaration| {
            let Declaration::Declaration {
                declaration_specifiers,
                ..
            } = decl
            else {
                panic!("expected declaration");
            };
            let spec = declaration_specifiers
                .iter()
                .find_map(|(spec, _)| match spec {
                    DeclarationSpecifier::TypeSpecifier((
                        TypeSpecifier::StructOrUnion { specifier, .. },
                        _,
                    )) => Some(specifier),
                    _ => None,
                });
            spec.and_then(|s| s.canonical_field_set_key())
                .expect("expected canonical key")
        };

        let key_a = key_of(&parsed_a.0.items[0].0);
        let key_b = key_of(&parsed_b.0.items[0].0);
        assert_eq!(key_a, key_b);
    }
}
