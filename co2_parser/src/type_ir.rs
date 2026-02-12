use chumsky::span::SimpleSpan;

use crate::{
    Span, Spanned,
    diagnostic::{raise_error, todo_error},
    parser as ast,
};

pub trait TypeInterface: Sized {
    fn mk_function(inputs: &[Self], output: Self) -> Self;
    fn mk_void() -> Self;
    fn mk_int(size: usize) -> Self;
    fn mk_ptr(inner: Self, span: Span) -> Self;
    fn mk_array(inner: Self, span: Span) -> Self;
    fn mk_typedef(path: ast::RustPath) -> Self;
}

pub fn base_type_of_decl<Ty: TypeInterface>(
    specifiers: Vec<Spanned<ast::DeclarationSpecifier>>,
    span: SimpleSpan,
) -> Ty {
    for (specifier, span) in &specifiers {
        match specifier {
            ast::DeclarationSpecifier::TypeSpecifier(type_specifier) => match &type_specifier.0 {
                ast::TypeSpecifier::Int => return Ty::mk_int(4),
                ast::TypeSpecifier::Void => return Ty::mk_void(),
                ast::TypeSpecifier::Char => return Ty::mk_int(1),
                ast::TypeSpecifier::Short => return Ty::mk_int(2),
                ast::TypeSpecifier::Long => (),
                ast::TypeSpecifier::Float => todo_error(*span),
                ast::TypeSpecifier::Double => todo_error(*span),
                ast::TypeSpecifier::Signed => (),
                ast::TypeSpecifier::Unsigned => (),
                ast::TypeSpecifier::StructOrUnion { kind: _, specifier } => {
                    todo!()
                    // return (
                    //     match specifier {
                    //         ast::StructOrUnionSpecifier::Defined { ident, fields } => {
                    //             let fields = self.convert_fields(fields);
                    //             self.add_item(
                    //                 Item::Struct {
                    //                     name: ident.clone(),
                    //                     fields,
                    //                 },
                    //                 *span,
                    //             );
                    //             RustType::TypeDef(RustPath::from_ident(ident.clone()))
                    //         }
                    //         ast::StructOrUnionSpecifier::Declared { ident } => {
                    //             RustType::TypeDef(RustPath::from_ident(ident.clone()))
                    //         }
                    //         ast::StructOrUnionSpecifier::Anonymous { fields } => {
                    //             let name =
                    //                 format!("__co2_anonymous_{}", self.anonymous_datatype_counter);
                    //             self.anonymous_datatype_counter += 1;
                    //             let fields = self.convert_fields(fields);
                    //             let name = (name.clone(), *span);
                    //             self.add_item(
                    //                 Item::Struct {
                    //                     name: name.clone(),
                    //                     fields,
                    //                 },
                    //                 *span,
                    //             );
                    //             RustType::TypeDef(RustPath::from_ident(name))
                    //         }
                    //     },
                    //     *span,
                    // );
                }
                ast::TypeSpecifier::TypedefName(ident) => {
                    if specifiers
                        .iter()
                        .filter(|spec| {
                            matches!(spec.0, ast::DeclarationSpecifier::TypeSpecifier(_))
                        })
                        .count()
                        != 1
                    {
                        raise_error(*span, "Cannot accept two type specifier.");
                    }
                    return Ty::mk_typedef(ident.0.clone());
                }
            },
            ast::DeclarationSpecifier::TypeQualifier(_) => (),
            ast::DeclarationSpecifier::StorageSpecifier(_) => {
                raise_error(*span, "Storage specifier is invalid in this position")
            }
        }
    }
    raise_error(span, "No suitable type specifier found for this declarator");
}

pub fn extract_type_of_decl<Ty: TypeInterface>(
    base: Ty,
    (decl, span): Spanned<ast::Declarator>,
) -> (Ty, Option<Spanned<String>>) {
    match decl {
        ast::Declarator::Abstract => (base, None),
        ast::Declarator::Identifier(ident) => (base, Some(ident)),
        ast::Declarator::FunctionDeclarator {
            declarator,
            param_list,
        } => {
            let inputs = param_list
                .parameters
                .into_iter()
                .map(|param| {
                    let base = base_type_of_decl::<Ty>(param.0, span);
                    extract_type_of_decl::<Ty>(base, param.1).0
                })
                .collect::<Vec<_>>();
            let (output, name) = extract_type_of_decl(base, *declarator);
            (Ty::mk_function(&inputs, output), name)
        }
        ast::Declarator::PointerDeclarator {
            declarator,
            qualifiers: _,
        } => {
            let (ty, name) = extract_type_of_decl(base, *declarator);
            (Ty::mk_ptr(ty, span), name)
        }
        ast::Declarator::ArrayDeclarator {
            declarator,
            subscription: _,
        } => {
            let (ty, name) = extract_type_of_decl(base, *declarator);
            (Ty::mk_array(ty, span), name)
        }
    }
}
