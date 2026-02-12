use std::{collections::HashMap, fmt::Display};

use chumsky::span::SimpleSpan;

use crate::{
    Span, Spanned,
    diagnostic::{raise_error, todo_error},
    parser::{self as ast, DeclarationSpecifier, LazyCompoundStatement, RustPath, UseItem},
    type_ir,
};
use itertools::Itertools;

#[derive(Debug, Clone)]
pub enum RustType {
    Void,
    Int(usize),
    TypeDef(RustPath),
    Function(FnSig),
    Ptr(Box<Spanned<RustType>>),
    Array(Box<Spanned<RustType>>),
}

impl type_ir::TypeInterface for RustType {
    fn mk_function(inputs: &[Self], output: Self) -> Self {
        RustType::Function(FnSig {
            inputs: inputs.to_vec(),
            output: Box::new(output),
        })
    }

    fn mk_void() -> Self {
        RustType::Void
    }

    fn mk_int(size: usize) -> Self {
        RustType::Int(size)
    }

    fn mk_ptr(inner: Self, span: Span) -> Self {
        RustType::Ptr(Box::new((inner, span)))
    }

    fn mk_array(inner: Self, span: Span) -> Self {
        RustType::Array(Box::new((inner, span)))
    }

    fn mk_typedef(path: RustPath) -> Self {
        RustType::TypeDef(path)
    }
}

impl Display for RustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustType::Void => write!(f, "void"),
            RustType::Int(n) => write!(f, "i{}", *n * 8),
            RustType::TypeDef(name) => write!(f, "{name}"),
            RustType::Function(fn_sig) => {
                write!(
                    f,
                    "fn ({}) -> {}",
                    fn_sig.inputs.iter().join(", "),
                    fn_sig.output
                )
            }
            RustType::Ptr(inner) => write!(f, "*mut {}", inner.0),
            RustType::Array(inner) => write!(f, "[{}; 101]", inner.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub inputs: Vec<RustType>,
    pub output: Box<RustType>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Use(UseItem),
    Function {
        name: Spanned<String>,
        sig: FnSig,
        body: LazyCompoundStatement,
    },
    ExternFunction {
        name: Spanned<String>,
        sig: FnSig,
    },
    TypeDef {
        name: Spanned<String>,
        value: Spanned<RustType>,
    },
    Static {
        name: Spanned<String>,
        ty: Spanned<RustType>,
    },
    Struct {
        name: Spanned<String>,
        fields: Vec<Field>,
    },
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Spanned<String>,
    pub ty: Spanned<RustType>,
}

impl Item {
    fn name(&self) -> Option<&str> {
        match self {
            Item::Use(_) => None,
            Item::Function { name, .. }
            | Item::ExternFunction { name, sig: _ }
            | Item::TypeDef { name, value: _ }
            | Item::Struct { name, fields: _ }
            | Item::Static { name, ty: _ } => Some(&name.0),
        }
    }
}

#[derive(Debug, Default)]
pub struct State {
    pub items: Vec<Spanned<Item>>,
    item_indexes_by_name: HashMap<String, usize>,
    anonymous_datatype_counter: usize,
}

impl State {
    fn convert_fields(&mut self, fields: &[Spanned<ast::StructOrUnionField>]) -> Vec<Field> {
        fields
            .iter()
            .flat_map(|(field, span)| {
                let specifiers = field
                    .specifiers
                    .iter()
                    .cloned()
                    .map(|(item, span)| (DeclarationSpecifier::TypeSpecifier((item, span)), span))
                    .collect();
                let base = &self.base_type_of_decl(specifiers, *span);
                let this = &mut *self;
                field
                    .declarators
                    .iter()
                    .map(move |decl| {
                        let (ty, name) =
                            this.extract_type_of_decl(base.clone(), decl.0.declarator.clone());
                        let name = name.unwrap();
                        Field { name, ty }
                    })
                    .collect_vec()
            })
            .collect()
    }

    pub fn collect_translation_unit(&mut self, tu: ast::TranslationUnit) {
        for use_item in tu.rust_use_items {
            self.add_item(Item::Use(use_item.0), use_item.1);
        }
        for item in tu.items {
            self.collect_declaration(item);
        }
    }

    fn collect_declaration(&mut self, (decl, span): Spanned<ast::Declaration>) {
        match decl {
            ast::Declaration::FunctionDefinition {
                declarator,
                declaration_specifiers,
                body,
            } => {
                let base = self.base_type_of_decl(declaration_specifiers, span);
                let (rust_type, name) = self.extract_type_of_decl(base, declarator);
                let Some(name) = name else {
                    raise_error(
                        span,
                        "Function definitions can not use abstract declarators.",
                    );
                };
                let RustType::Function(sig) = rust_type.0 else {
                    raise_error(span, "Invalid type for function");
                };
                let body = body.0;
                self.add_item(Item::Function { name, sig, body }, span);
            }
            ast::Declaration::Declaration {
                mut declaration_specifiers,
                declarators,
            } => {
                let mut is_typedef = None;
                declaration_specifiers.retain(|(elem, span)| {
                    if let DeclarationSpecifier::StorageSpecifier((storage_specifier, _)) = elem {
                        match storage_specifier {
                            ast::StorageClassSpecifier::Typedef => {
                                is_typedef = Some(*span);
                                return false;
                            }
                            ast::StorageClassSpecifier::Extern
                            | ast::StorageClassSpecifier::Static
                            | ast::StorageClassSpecifier::ThreadLocal
                            | ast::StorageClassSpecifier::Auto
                            | ast::StorageClassSpecifier::Register => return false,
                        }
                    }
                    true
                });
                let base_type = self.base_type_of_decl(declaration_specifiers, span);
                for declarator in declarators {
                    let (rust_type, name) =
                        self.extract_type_of_decl(base_type.clone(), declarator.0.declarator);
                    let Some(name) = name else {
                        continue;
                    };
                    let item = if is_typedef.is_some() {
                        Item::TypeDef {
                            name,
                            value: rust_type,
                        }
                    } else {
                        if let RustType::Function(sig) = rust_type.0 {
                            Item::ExternFunction { name, sig }
                        } else {
                            Item::Static {
                                name,
                                ty: rust_type,
                            }
                        }
                    };
                    self.add_item(item, span);
                }
            }
        }
    }

    fn add_item(&mut self, item: Item, span: Span) {
        let item = (item, span);
        let Some(name) = item.0.name() else {
            self.items.push(item);
            return;
        };
        match self.item_indexes_by_name.entry(name.to_owned()) {
            std::collections::hash_map::Entry::Occupied(index) => {
                self.items[*index.get()] = item;
            }
            std::collections::hash_map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(self.items.len());
                self.items.push(item);
            }
        }
    }

    fn extract_type_of_decl(
        &self,
        base: Spanned<RustType>,
        decl: Spanned<ast::Declarator>,
    ) -> (Spanned<RustType>, Option<Spanned<String>>) {
        let (ty, decl) = type_ir::extract_type_of_decl::<RustType>(base.0, decl);
        ((ty, base.1), decl)
    }

    fn base_type_of_decl(
        &self,
        specifiers: Vec<Spanned<ast::DeclarationSpecifier>>,
        span: SimpleSpan,
    ) -> Spanned<RustType> {
        let ty = type_ir::base_type_of_decl::<RustType>(specifiers, span);
        (ty, span)
    }
}
