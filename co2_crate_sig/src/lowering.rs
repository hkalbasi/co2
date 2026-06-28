use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::RandomState},
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use co2_ast::{
    Constant, Declaration, DeclarationSpecifier, Declarator, Designator, DoTransform as _,
    Expression, FunctionDefinitionSignature, InitDeclarator, Initializer, IntegerSuffix, ModItem,
    Rich, StatelessResolver, StorageClassSpecifier, StructOrUnionKind, StructOrUnionSpecifier,
    Token, TranslationUnit, TypeQualifier, TypeResolver, TypeSpecifier, Visibility as AstVisibility,
    co2_test_symbol_name,
};
use co2_parser::{
    parse_compound_statement, parse_translation_unit_from_preprocessed,
    parse_translation_unit_from_tokens,
};
use co2_preprocessor::PreprocessedSource;
use rustc_public_generative::{
    AdtRepr, DefData, FileId, ForeignModItem, FunctionAbi, FunctionSignature, GeneratedAttr,
    HirAdtKind, HirGenericArg, HirImplItem, HirImplItemKind, HirLifetime, HirModule, HirModuleItem,
    HirSelfKind, HirStructure, HirStructureCtx, HirTy, HirTyConst, HirTyKind, InlineHint,
    StructField, Visibility,
    rustc_public::{
        DefId,
        mir::Mutability,
        ty::{AdtDef, FnDef, IntTy, RigidTy, Ty, UintTy},
    },
};

use crate::{
    CrateSigCtx, DefOrLocal, LocalResolver, LocalResolverBase, MirOwnerInfo,
    ast_resolver::StructAndEnumData,
    resolver::{ModuleData, Resolver},
    struct_manager::{PendingEnum, StructData, StructManager},
    ty::CTy,
};

#[derive(Clone, Copy)]
pub struct WellknownDefs {
    pub maybe_uninit: AdtDef,
    pub maybe_uninit_uninit: FnDef,
    pub valist: AdtDef,
    pub valist_fn_arg: FnDef,
    pub clone: FnDef,
    pub transmute: FnDef,
    pub transmute_copy: FnDef,
    pub offset_mut: FnDef,
    pub offset_const: FnDef,
    pub offset_from: FnDef,
    pub zeroed: FnDef,
}

fn has_const_qualifier_in_decl_specs(
    specs: &[co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>],
) -> bool {
    specs.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Const, _))
        )
    })
}

fn lower_generated_attrs(attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>]) -> Vec<GeneratedAttr> {
    attrs
        .iter()
        .filter_map(|(attr, _)| {
            let path = attr
                .path
                .iter()
                .map(|seg| seg.0.clone())
                .collect::<Vec<_>>();
            if path == ["doc"] {
                let doc_text = match attr.args.as_slice() {
                    [(co2_ast::Token::StringLit(lit), _)]
                    | [
                        (co2_ast::Token::Assign, _),
                        (co2_ast::Token::StringLit(lit), _),
                    ] => Some(lit.bytes.clone()),
                    _ => None,
                }?;
                Some(GeneratedAttr::DocComment {
                    comment: String::from_utf8_lossy(&doc_text).into_owned(),
                    inner: attr.is_inner(),
                })
            } else if path == ["inline"] {
                let inner_args: &[co2_ast::Spanned<co2_ast::Token>] = match attr.args.as_slice() {
                    [
                        (co2_ast::Token::LParen, _),
                        inner @ ..,
                        (co2_ast::Token::RParen, _),
                    ] => inner,
                    other => other,
                };
                let hint = match inner_args {
                    [] => Some(InlineHint::Hint),
                    [(co2_ast::Token::Ident(s), _)] if s == "always" => Some(InlineHint::Always),
                    [(co2_ast::Token::Ident(s), _)] if s == "never" => Some(InlineHint::Never),
                    _ => None,
                };
                hint.map(GeneratedAttr::InlineHint)
            } else if attr.args.is_empty() {
                Some(GeneratedAttr::Word { path })
            } else {
                None
            }
        })
        .collect()
}

fn lower_module_file_attrs_for_decl_item(
    attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>],
) -> Vec<GeneratedAttr> {
    lower_generated_attrs(attrs)
        .into_iter()
        .map(|attr| match attr {
            GeneratedAttr::DocComment { comment, .. } => GeneratedAttr::DocComment {
                comment,
                inner: false,
            },
            GeneratedAttr::Word { path } => GeneratedAttr::Word { path },
            GeneratedAttr::InlineHint(hint) => GeneratedAttr::InlineHint(hint),
        })
        .collect()
}

fn has_derive_attr(attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>], trait_name: &str) -> bool {
    attrs.iter().any(|(attr, _)| {
        attr.path.len() == 1
            && attr.path[0].0 == "derive"
            && attr
                .args
                .iter()
                .any(|(tok, _)| matches!(tok, co2_ast::Token::Ident(s) if s == trait_name))
    })
}

fn is_known_fn_word_attr(name: &str) -> bool {
    matches!(
        name,
        "test" | "ignore" | "should_panic" | "no_mangle" | "inline"
    )
}

fn validate_attrs_for_fn(
    attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>],
    is_c_style: bool,
) -> Vec<co2_ast::Rich<'static, String, co2_ast::Span>> {
    let mut errors = Vec::new();
    for (attr, _) in attrs {
        let path: Vec<&str> = attr.path.iter().map(|seg| seg.0.as_str()).collect();
        match path.as_slice() {
            ["doc"] => {}
            ["no_mangle"] if is_c_style => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    "C style functions are already no_mangle, remove this.",
                ));
            }
            ["derive"] => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    "`derive` is not applicable to functions",
                ));
            }
            ["repr"] => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    "`repr` is not applicable to functions",
                ));
            }
            ["inline"] => {
                for (tok, span) in &attr.args {
                    if let co2_ast::Token::Ident(s) = tok
                        && s != "always"
                        && s != "never"
                    {
                        errors.push(co2_ast::Rich::custom(
                            *span,
                            format!("unknown inline hint `{s}`"),
                        ));
                    }
                }
            }
            [name] if attr.args.is_empty() && !is_known_fn_word_attr(name) => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    format!("unknown attribute `{name}`"),
                ));
            }
            _ => {}
        }
    }
    errors
}

fn validate_attrs_for_struct(
    attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>],
) -> Vec<co2_ast::Rich<'static, String, co2_ast::Span>> {
    let mut errors = Vec::new();
    for (attr, _) in attrs {
        let path: Vec<&str> = attr.path.iter().map(|seg| seg.0.as_str()).collect();
        match path.as_slice() {
            ["derive"] if attr.args.len() >= 2 => {
                let inner = &attr.args[1..attr.args.len() - 1];
                for (token, span) in inner {
                    if let co2_ast::Token::Ident(name) = token
                        && !matches!(name.as_str(), "Copy" | "Clone")
                    {
                        errors.push(co2_ast::Rich::custom(
                            *span,
                            format!("unknown derive macro `{name}`"),
                        ));
                    }
                }
            }
            ["repr"] if attr.args.len() >= 2 => {
                let inner = &attr.args[1..attr.args.len() - 1];
                for (token, span) in inner {
                    if let co2_ast::Token::Ident(name) = token
                        && !matches!(name.as_str(), "C" | "Rust" | "packed")
                    {
                        errors.push(co2_ast::Rich::custom(
                            *span,
                            format!("unrecognized representation hint `{name}`"),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    errors
}

fn collect_attr_errors_from_tu(
    tu: &co2_ast::TranslationUnit<co2_ast::StatelessResolver>,
) -> Vec<co2_ast::Rich<'static, String, co2_ast::Span>> {
    let mut errors = Vec::new();
    for (item, _) in &tu.items {
        match item {
            co2_ast::Declaration::FunctionDefinition {
                attrs: decl_attrs,
                signature,
                ..
            } => {
                let is_c_style =
                    matches!(signature, co2_ast::FunctionDefinitionSignature::C { .. });
                errors.extend(validate_attrs_for_fn(decl_attrs, is_c_style));
                if let co2_ast::FunctionDefinitionSignature::Rust(sig) = signature {
                    errors.extend(validate_attrs_for_fn(&sig.attrs, false));
                }
            }
            co2_ast::Declaration::RustStruct { attrs, .. } => {
                errors.extend(validate_attrs_for_struct(attrs));
            }
            _ => {}
        }
    }
    errors
}

fn validate_all_attrs(
    tu: &co2_ast::TranslationUnit<co2_ast::StatelessResolver>,
    modules: &[LoadedModule],
) {
    let mut errors = collect_attr_errors_from_tu(tu);
    for module in modules {
        errors.extend(validate_module_attrs_recursive(module));
    }
    if !errors.is_empty() {
        co2_ast::emit_errors_and_terminate(errors);
    }
}

fn validate_attrs_for_mod(
    attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>],
) -> Vec<co2_ast::Rich<'static, String, co2_ast::Span>> {
    let mut errors = Vec::new();
    for (attr, _) in attrs {
        let path: Vec<&str> = attr.path.iter().map(|seg| seg.0.as_str()).collect();
        match path.as_slice() {
            ["doc"] => {}
            ["derive"] => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    "`derive` is not applicable to modules",
                ));
            }
            ["repr"] => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    "`repr` is not applicable to modules",
                ));
            }
            [name] if attr.args.is_empty() && !is_known_fn_word_attr(name) => {
                errors.push(co2_ast::Rich::custom(
                    attr.path[0].1,
                    format!("unknown attribute `{name}`"),
                ));
            }
            _ => {}
        }
    }
    errors
}

fn validate_module_attrs_recursive(
    module: &LoadedModule,
) -> Vec<co2_ast::Rich<'static, String, co2_ast::Span>> {
    let mut errors = validate_attrs_for_mod(&module.attrs);
    errors.extend(collect_attr_errors_from_tu(&module.tu));
    for child in &module.children {
        errors.extend(validate_module_attrs_recursive(child));
    }
    errors
}

fn has_word_attr(attrs: &[GeneratedAttr], word: &str) -> bool {
    attrs
        .iter()
        .any(|attr| matches!(attr, GeneratedAttr::Word { path } if path.as_slice() == [word]))
}

fn constexpr_decl_span(
    specs: &[co2_ast::Spanned<DeclarationSpecifier<LocalResolver>>],
    fallback: co2_ast::Span,
) -> co2_ast::Span {
    specs
        .iter()
        .find_map(|(spec, span)| spec.is_constexpr().then_some(*span))
        .unwrap_or(fallback)
}

fn is_scalar_type(ty: &HirTy) -> bool {
    matches!(
        &ty.kind,
        HirTyKind::Bool
            | HirTyKind::Char
            | HirTyKind::Int(..)
            | HirTyKind::Uint(..)
            | HirTyKind::Float(..)
    )
}

fn expr_contains_local(expr: &Expression<LocalResolver>) -> bool {
    match expr {
        Expression::Identifier((resolved, _)) => {
            matches!(resolved, DefOrLocal::Local(_) | DefOrLocal::LocalConst(_))
        }
        Expression::Field(base, _)
        | Expression::Arrow(base, _)
        | Expression::Update { expr: base, .. }
        | Expression::Sizeof(base)
        | Expression::Alignof(base)
        | Expression::UnaryOp(_, base)
        | Expression::BuiltinConstantP { expr: base } => expr_contains_local(&base.0),
        Expression::Subscript(base, index) => {
            expr_contains_local(&base.0) || expr_contains_local(&index.0)
        }
        Expression::Call { func, params } => {
            expr_contains_local(&func.0) || params.iter().any(|param| expr_contains_local(&param.0))
        }
        Expression::MethodCall {
            receiver, params, ..
        } => {
            expr_contains_local(&receiver.0)
                || params.iter().any(|param| expr_contains_local(&param.0))
        }
        Expression::AssignWithOp { lhs, rhs, .. } | Expression::BinOp(lhs, _, rhs) => {
            expr_contains_local(&lhs.0) || expr_contains_local(&rhs.0)
        }
        Expression::Cast { type_name, expr } => {
            type_name_contains_local(type_name) || expr_contains_local(&expr.0)
        }
        Expression::SizeofType(type_name)
        | Expression::AlignofType(type_name)
        | Expression::Offsetof { ty: type_name, .. } => type_name_contains_local(type_name),
        Expression::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_local(&cond.0)
                || expr_contains_local(&then_expr.0)
                || expr_contains_local(&else_expr.0)
        }
        Expression::CompoundLiteral {
            type_name,
            initializer,
        } => type_name_contains_local(type_name) || initializer_contains_local(&initializer.0),
        Expression::VaStart { args, .. }
        | Expression::VaArg { args, .. }
        | Expression::VaEnd { args } => expr_contains_local(&args.0),
        Expression::VaCopy { dest, src } => {
            expr_contains_local(&dest.0) || expr_contains_local(&src.0)
        }
        Expression::GenericSelection {
            controlling,
            associations,
        } => {
            expr_contains_local(&controlling.0)
                || associations.iter().any(|(assoc, _)| match assoc {
                    co2_ast::GenericAssociation::Type { type_name, expr } => {
                        type_name_contains_local(type_name) || expr_contains_local(&expr.0)
                    }
                    co2_ast::GenericAssociation::Default { expr } => expr_contains_local(&expr.0),
                })
        }
        Expression::BuiltinTypesCompatibleP { ty1, ty2 } => {
            type_name_contains_local(ty1) || type_name_contains_local(ty2)
        }
        Expression::Empty
        | Expression::Constant(_)
        | Expression::LabelAddress(_)
        | Expression::GnuStatementExpr { .. } => false,
    }
}

fn initializer_contains_local(initializer: &Initializer<LocalResolver>) -> bool {
    match initializer {
        Initializer::Expr(expr) => expr_contains_local(&expr.0),
        Initializer::List(items) => items.iter().any(|(item, _)| {
            item.designators.as_ref().is_some_and(|designators| {
                designators.iter().any(|(designator, _)| match designator {
                    Designator::Subscript(expr) => expr_contains_local(&expr.0),
                    Designator::Range(start, end) => {
                        expr_contains_local(&start.0) || expr_contains_local(&end.0)
                    }
                    Designator::Field(_) => false,
                })
            }) || initializer_contains_local(&item.initializer.0)
        }),
    }
}

fn type_name_contains_local(type_name: &co2_ast::TypeName<LocalResolver>) -> bool {
    type_name
        .specifier_qualifier_list
        .iter()
        .any(|(specifier, _)| match specifier {
            co2_ast::SpecifierQualifier::TypeSpecifier((ty, _)) => match ty {
                co2_ast::TypeSpecifier::TypeofExpr(expr) => expr_contains_local(&expr.0),
                co2_ast::TypeSpecifier::TypeofType(type_name) => {
                    type_name_contains_local(type_name)
                }
                _ => false,
            },
            co2_ast::SpecifierQualifier::TypeQualifier(_) => false,
        })
        || type_name
            .abstract_declarator
            .as_ref()
            .is_some_and(|(declarator, _)| declarator_contains_local(declarator))
}

fn declarator_contains_local(declarator: &Declarator<LocalResolver>) -> bool {
    match declarator {
        Declarator::Abstract | Declarator::Identifier(_) => false,
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. } => {
            declarator_contains_local(&declarator.0)
        }
        Declarator::ArrayDeclarator {
            declarator,
            subscription: _,
        } => declarator_contains_local(&declarator.0),
    }
}

fn declarator_ident_span<R: co2_ast::TypeResolver>(
    declarator: &co2_ast::Spanned<Declarator<R>>,
) -> Option<co2_ast::Span> {
    match &declarator.0 {
        Declarator::Identifier((_, span)) => Some(*span),
        Declarator::FunctionDeclarator { declarator, .. }
        | Declarator::PointerDeclarator { declarator, .. }
        | Declarator::ArrayDeclarator { declarator, .. } => declarator_ident_span(declarator),
        Declarator::Abstract => None,
    }
}

fn deduplicate_tu_items(
    mut tu: TranslationUnit<StatelessResolver>,
) -> TranslationUnit<StatelessResolver> {
    #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
    enum TuItemKind {
        ExternVarDeclOrFuncDecl,
        StaticUninitOrFuncDecl,
        ExternFunction,
        ExternVarDecl,
        StaticUninit,
        Typedef,
        FunctionDeclaration,
        StaticInitialized,
        FunctionDefinition,
        RustFunctionDefinition,
        RustTypeAlias,
        RustStruct,
    }
    use TuItemKind::*;
    impl TuItemKind {
        fn check_mergeable(self, other: Self) -> Option<Self> {
            if self < other {
                return other.check_mergeable(self);
            }
            match (self, other) {
                (Typedef, Typedef) => Some(Typedef),
                (
                    FunctionDefinition,
                    ExternFunction
                    | FunctionDeclaration
                    | ExternVarDeclOrFuncDecl
                    | StaticUninitOrFuncDecl,
                ) => Some(FunctionDefinition),
                (
                    FunctionDeclaration,
                    ExternFunction
                    | FunctionDeclaration
                    | ExternVarDeclOrFuncDecl
                    | StaticUninitOrFuncDecl,
                ) => Some(FunctionDeclaration),
                (ExternFunction, ExternFunction) => Some(ExternFunction),
                (
                    ExternVarDecl,
                    ExternVarDecl | ExternVarDeclOrFuncDecl | StaticUninitOrFuncDecl,
                ) => Some(ExternVarDecl),
                (
                    StaticInitialized,
                    StaticUninit | ExternVarDecl | ExternVarDeclOrFuncDecl | StaticUninitOrFuncDecl,
                ) => Some(StaticInitialized),
                (StaticUninit, StaticUninit | ExternVarDecl) => Some(StaticUninit),
                _ => None,
            }
        }
    }

    let mut errors: Vec<co2_ast::Rich<'_, String, co2_ast::Span>> = Vec::new();
    let mut tu_item_id: usize = 0;
    let mut name_to_important_def = HashMap::<String, (usize, TuItemKind)>::new();

    for (item, _) in &tu.items {
        match item {
            Declaration::FunctionDefinition { signature, .. } => {
                let name = signature.ident().unwrap();
                let kind = match signature {
                    FunctionDefinitionSignature::C { .. } => FunctionDefinition,
                    FunctionDefinitionSignature::Rust(_) => RustFunctionDefinition,
                };
                match name_to_important_def.entry(name) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let (_, old_kind) = *entry.get();
                        if old_kind.check_mergeable(kind).is_none() {
                            errors.push(co2_ast::Rich::custom(
                                signature.ident_span().unwrap(),
                                format!(
                                    "the name `{}` is defined multiple times",
                                    signature.ident().unwrap()
                                ),
                            ));
                        }
                        if kind > old_kind {
                            entry.insert((tu_item_id, kind));
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((tu_item_id, kind));
                    }
                }
                tu_item_id += 1;
            }
            Declaration::RustTypeAlias { ident, .. } | Declaration::RustStruct { ident, .. } => {
                let name = ident.0.clone();
                let kind = match item {
                    Declaration::RustTypeAlias { .. } => RustTypeAlias,
                    _ => RustStruct,
                };
                match name_to_important_def.entry(name) {
                    std::collections::hash_map::Entry::Occupied(entry) => {
                        let (_, old_kind) = *entry.get();
                        if old_kind.check_mergeable(kind).is_none() {
                            errors.push(co2_ast::Rich::custom(
                                ident.1,
                                format!("the name `{}` is defined multiple times", ident.0),
                            ));
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((tu_item_id, kind));
                    }
                }
                tu_item_id += 1;
            }
            Declaration::Declaration {
                attrs: _,
                declaration_specifiers,
                declarators,
            } => {
                let is_typedef = declaration_specifiers.iter().any(|x| x.0.is_typedef());
                let is_extern = declaration_specifiers.iter().any(|x| x.0.is_extern());
                let uses_typedef_name = !is_typedef
                    && declaration_specifiers.iter().any(|spec| {
                        matches!(
                            &spec.0,
                            co2_ast::DeclarationSpecifier::TypeSpecifier(ts)
                                if matches!(&ts.0, co2_ast::TypeSpecifier::TypedefName(..))
                        )
                    });
                for decl in declarators {
                    let name = decl.0.declarator.0.ident().unwrap();
                    let kind = if is_typedef {
                        Typedef
                    } else if decl.0.declarator.0.is_function() {
                        if is_extern {
                            ExternFunction
                        } else {
                            FunctionDeclaration
                        }
                    } else if decl.0.initializer.is_some() {
                        StaticInitialized
                    } else if uses_typedef_name {
                        if is_extern {
                            ExternVarDeclOrFuncDecl
                        } else {
                            StaticUninitOrFuncDecl
                        }
                    } else if is_extern {
                        ExternVarDecl
                    } else {
                        StaticUninit
                    };
                    match name_to_important_def.entry(name.clone()) {
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            let (_, old_kind) = *entry.get();
                            if old_kind.check_mergeable(kind).is_none() {
                                let err_span = decl.0.declarator.0.ident_span().unwrap();
                                errors.push(co2_ast::Rich::custom(
                                    err_span,
                                    format!("the name `{name}` is defined multiple times"),
                                ));
                            }
                            if kind > old_kind {
                                entry.insert((tu_item_id, kind));
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert((tu_item_id, kind));
                        }
                    }
                    tu_item_id += 1;
                }
            }
            Declaration::PragmaPack { .. } | Declaration::BreakCo2 => {}
        }
    }

    tu_item_id = 0;

    tu.items.retain_mut(|item| match &mut item.0 {
        Declaration::FunctionDefinition { signature, .. } => {
            let name = signature.ident().unwrap();
            let is_needed = name_to_important_def[&name].0 == tu_item_id;
            tu_item_id += 1;
            is_needed
        }
        Declaration::RustTypeAlias { .. } | Declaration::RustStruct { .. } => {
            tu_item_id += 1;
            true
        }
        Declaration::Declaration {
            attrs: _,
            declaration_specifiers: _,
            declarators,
        } => {
            declarators.retain(|decl| {
                let name = decl.0.declarator.0.ident().unwrap();
                let is_needed = name_to_important_def[&name].0 == tu_item_id;
                tu_item_id += 1;
                is_needed
            });
            true
        }
        Declaration::PragmaPack { .. } | Declaration::BreakCo2 => true,
    });

    if !errors.is_empty() {
        co2_ast::emit_errors_and_terminate(errors);
    }

    tu
}

#[derive(Clone)]
struct LoadedModule {
    name: String,
    def_id: DefId,
    attrs: Vec<co2_ast::Spanned<co2_ast::RustAttribute>>,
    decl_span: co2_ast::Span,
    source_name: String,
    source: &'static str,
    tu: TranslationUnit<StatelessResolver>,
    children: Vec<LoadedModule>,
}

struct SourceMapSnapshot {
    files: Arc<HashMap<co2_ast::FileId, (String, Arc<str>)>>,
}

impl co2_ast::SourceMap for SourceMapSnapshot {
    fn get_file_info(&self, id: co2_ast::FileId) -> Option<(String, Arc<str>)> {
        self.files.get(&id).cloned()
    }
}

fn root_module_dir(source_path: &Path) -> PathBuf {
    source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn child_module_dir(source_path: &Path) -> PathBuf {
    if source_path.file_stem().and_then(|stem| stem.to_str()) == Some("mod") {
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source_path.file_stem().unwrap_or_default())
    }
}

fn resolve_module_source(module_dir: &Path, module_name: &str) -> Option<PathBuf> {
    let direct = module_dir.join(format!("{module_name}.co2"));
    if direct.is_file() {
        return Some(direct);
    }

    let nested = module_dir.join(module_name).join("mod.co2");
    if nested.is_file() {
        return Some(nested);
    }

    None
}

fn register_preprocessed_files(
    ctx: &HirStructureCtx<'_>,
    preprocessed: &PreprocessedSource,
    rustc_file_ids: &mut HashMap<co2_ast::FileId, FileId>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>)>,
) {
    for (file_id, file) in preprocessed.files() {
        source_files
            .entry(*file_id)
            .or_insert_with(|| (file.path.display().to_string(), file.source.clone()));
        rustc_file_ids
            .entry(*file_id)
            .or_insert_with(|| ctx.add_custom_file(&file.path, file.source.as_ref()));
    }
}

fn load_modules(
    ctx: &HirStructureCtx<'_>,
    parent_def: DefId,
    module_dir: &Path,
    rust_mod_items: &[co2_ast::Spanned<co2_ast::ModItem>],
    rustc_file_ids: &mut HashMap<co2_ast::FileId, FileId>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>)>,
    loaded_paths: &mut HashSet<PathBuf>,
) -> Result<Vec<LoadedModule>, ModItem> {
    let mut modules = Vec::with_capacity(rust_mod_items.len());

    for (mod_item, mod_span) in rust_mod_items {
        let def_id = ctx.allocate_def_id(parent_def, &DefData::Module(mod_item.name.0.clone()));

        if let Some((inline_tokens, end_span)) = &mod_item.inline_content {
            // Inline module: parse from the already-captured tokens.
            // The tokens' spans already reference the parent file, so no new
            // preprocessed source needs to be registered.
            let source_name = format!("<inline module '{}'>", mod_item.name.0);
            let tu = parse_translation_unit_from_tokens(
                inline_tokens,
                &source_name,
                "",
                *end_span,
                StatelessResolver::new(),
            )
            .0;
            let tu = deduplicate_tu_items(tu);
            // Child file-based modules are resolved relative to the inline module's
            // virtual directory (same convention as Rust: parent_dir/mod_name/).
            let child_dir = module_dir.join(&mod_item.name.0);
            let children = load_modules(
                ctx,
                def_id,
                &child_dir,
                &tu.rust_mod_items,
                rustc_file_ids,
                source_files,
                loaded_paths,
            )?;

            modules.push(LoadedModule {
                name: mod_item.name.0.clone(),
                def_id,
                attrs: mod_item.attrs.clone(),
                decl_span: *mod_span,
                source_name,
                source: "",
                tu,
                children,
            });
        } else {
            // File-based module: load from disk.
            let Some(module_path) = resolve_module_source(module_dir, &mod_item.name.0) else {
                return Err(mod_item.clone());
            };
            assert!(
                loaded_paths.insert(module_path.clone()),
                "module loaded multiple times: {}",
                module_path.display()
            );

            let preprocessed = co2_preprocessor::preprocess(&module_path, &Vec::new());
            register_preprocessed_files(ctx, &preprocessed, rustc_file_ids, source_files);
            co2_ast::set_source_map(Arc::new(SourceMapSnapshot {
                files: Arc::new(source_files.clone()),
            }));

            let source_name = module_path.to_string_lossy().into_owned();
            let source: &'static str = Box::leak(preprocessed.raw_src.to_string().into_boxed_str());
            let tu = parse_translation_unit_from_preprocessed(
                &source_name,
                &preprocessed,
                StatelessResolver::new(),
            )
            .expect("failed to parse co2 module")
            .0;
            let tu = deduplicate_tu_items(tu);
            let children = load_modules(
                ctx,
                def_id,
                &child_module_dir(&module_path),
                &tu.rust_mod_items,
                rustc_file_ids,
                source_files,
                loaded_paths,
            )?;

            modules.push(LoadedModule {
                name: mod_item.name.0.clone(),
                def_id,
                attrs: mod_item.attrs.clone(),
                decl_span: *mod_span,
                source_name,
                source,
                tu,
                children,
            });
        }
    }

    Ok(modules)
}

fn build_module_data_tree(
    ctx: &HirStructureCtx<'_>,
    module: &LoadedModule,
    foreign_mod: DefId,
    module_path: &[String],
    test: bool,
) -> ModuleData {
    let mut data = ModuleData::forward_pass_parsed_module(
        ctx,
        &module.tu,
        module.def_id,
        foreign_mod,
        module_path,
        false,
        test,
    );
    for child in &module.children {
        let mut child_path = module_path.to_vec();
        child_path.push(child.name.clone());
        data.insert_alias(
            &child.name,
            build_module_data_tree(ctx, child, foreign_mod, &child_path, test),
        );
    }
    data
}

fn import_module_use_items(
    resolver: &mut Resolver,
    module_path: &[String],
    modules: &[LoadedModule],
) {
    for module in modules {
        let mut child_path = module_path.to_vec();
        child_path.push(module.name.clone());
        resolver.import_use_items(&child_path, &module.tu);
        import_module_use_items(resolver, &child_path, &module.children);
    }
}

fn resolve_in_module<'a>(
    ctx: &CrateSigCtx<'_>,
    module_path: &'a [String],
    name: &'a str,
) -> (DefId, co2_ast::TypeQueryResult) {
    ctx.resolve_in_current(
        module_path
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name)),
    )
    .unwrap()
}

fn adt_repr_from_attrs(attrs: &[co2_ast::Spanned<co2_ast::RustAttribute>]) -> AdtRepr {
    for (attr, _) in attrs {
        if attr.path.len() == 1 && attr.path[0].0 == "repr" && attr.args.len() >= 2 {
            let inner = &attr.args[1..attr.args.len() - 1];
            let mut repr = AdtRepr::Rust;
            let mut packed = false;
            let mut pack_align: Option<u32> = None;
            let mut i = 0;
            while i < inner.len() {
                match &inner[i].0 {
                    Token::Ident(s) if s == "C" => repr = AdtRepr::C,
                    Token::Ident(s) if s == "Rust" => repr = AdtRepr::Rust,
                    Token::Ident(s) if s == "packed" => {
                        packed = true;
                        if i + 2 < inner.len()
                            && matches!(&inner[i + 1].0, Token::LParen)
                            && let Some((Token::Integer(text, _), _)) = inner.get(i + 2)
                            && let Ok(n) = text.parse::<u32>()
                        {
                            pack_align = Some(n);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if let Some(align) = pack_align {
                return AdtRepr::CPacked(align);
            }
            if packed {
                return AdtRepr::CPacked(1);
            }
            return repr;
        }
    }
    AdtRepr::Rust
}

fn lower_translation_unit_items(
    ctx: &mut CrateSigCtx<'_>,
    tu: &TranslationUnit<StatelessResolver>,
    modules: &[LoadedModule],
    module_path: &[String],
    source_name: &str,
    source: &'static str,
    foreign_mod: DefId,
    foreign_items: &mut Vec<ForeignModItem>,
    no_main: bool,
    test: bool,
    on_body_parsed: &mut Option<&mut dyn FnMut(Duration)>,
) -> Vec<HirModuleItem> {
    _ = foreign_mod;
    let mut hir_items = Vec::new();
    for (item, parser_span) in tu.items.clone() {
        let span = ctx.co2_span_to_rustc(parser_span);
        let mut resolver =
            LocalResolver::new(ctx.resolver.clone()).with_module_path(module_path.to_vec());

        if let Declaration::Declaration {
            attrs: _,
            declaration_specifiers,
            declarators,
        } = &item
            && let [
                (DeclarationSpecifier::StorageSpecifier((StorageClassSpecifier::Typedef, _)), _),
                (
                    DeclarationSpecifier::TypeSpecifier((
                        TypeSpecifier::StructOrUnion { kind, specifier },
                        _,
                    )),
                    _,
                ),
            ] = declaration_specifiers.as_slice()
            && let StructOrUnionSpecifier::Anonymous { fields } = &specifier.0
            && let [
                (
                    InitDeclarator {
                        declarator,
                        initializer: None,
                        is_transparent_union: false,
                    },
                    _,
                ),
            ] = declarators.as_slice()
            && let Declarator::Identifier((name, _)) = &declarator.0
        {
            let type_def = resolve_in_module(ctx, module_path, name).0;
            let transformed_fields: Vec<_> = fields
                .iter()
                .map(|(f, sp)| (f.transform(&resolver), *sp))
                .collect();
            let mut base = ctx.resolver.borrow_mut();
            base.struct_manager.definitions.insert(
                type_def,
                StructData {
                    def_id: type_def,
                    name: name.clone(),
                    tag_name: Some(name.clone()),
                    kind: *kind,
                    span,
                    emitted_fields: None,
                    logical_fields: None,
                    pack_align: None,
                    skip_emit: false,
                },
            );
            base.define_def(type_def, &transformed_fields, span);
            let fields = base.struct_manager.definitions[&type_def]
                .emitted_fields
                .clone()
                .unwrap();
            let pack = base.struct_manager.definitions[&type_def].pack_align;
            base.struct_manager
                .definitions
                .get_mut(&type_def)
                .unwrap()
                .skip_emit = true;
            drop(base);
            let adt_kind = match kind {
                StructOrUnionKind::Struct => HirAdtKind::Struct { fields },
                StructOrUnionKind::Union => HirAdtKind::Union { fields },
            };
            let adt_repr = match pack {
                Some(n) => AdtRepr::CPacked(n),
                None => AdtRepr::C,
            };
            hir_items.push(HirModuleItem::Adt {
                name: name.clone(),
                id: AdtDef(type_def),
                kind: adt_kind,
                span,
                repr: adt_repr,
                visibility: Visibility::Public,
            });
            add_clone_and_copy_for_def(ctx, type_def, span);
            continue;
        }

        let item = item.transform(&resolver);
        match item {
            Declaration::RustTypeAlias {
                attrs,
                ident,
                ty,
                visibility: ast_vis,
            } => {
                let name = ident.0.1;
                let id = resolve_in_module(ctx, module_path, &name).0;
                hir_items.push(HirModuleItem::TypeDef {
                    name,
                    id,
                    ty: ctx.lower_rust_ty(ty),
                    attrs: lower_generated_attrs(&attrs),
                    visibility: match ast_vis {
                        AstVisibility::Public => Visibility::Public,
                        AstVisibility::Crate => Visibility::Crate,
                        AstVisibility::Restricted => Visibility::Restricted,
                        AstVisibility::Private => Visibility::Private,
                    },
                    span,
                });
            }
            Declaration::RustStruct {
                attrs,
                ident,
                fields: struct_fields,
                visibility: ast_vis,
            } => {
                let name = ident.0.1;
                let id = resolve_in_module(ctx, module_path, &name).0;
                let repr = adt_repr_from_attrs(&attrs);
                let fields = struct_fields
                    .into_iter()
                    .map(|field| {
                        let field_span = ctx.co2_span_to_rustc(field.name.1);
                        let field_ty = ctx.lower_rust_ty(field.ty);
                        let field_id =
                            ctx.allocate_def_id(id, &DefData::ValueNs(field.name.0.1.clone()));
                        StructField {
                            id: field_id,
                            name: field.name.0.1,
                            ty: field_ty,
                            span: field_span,
                        }
                    })
                    .collect();
                hir_items.push(HirModuleItem::Adt {
                    name,
                    id: AdtDef(id),
                    kind: HirAdtKind::Struct { fields },
                    visibility: match ast_vis {
                        AstVisibility::Public => Visibility::Public,
                        AstVisibility::Crate => Visibility::Crate,
                        AstVisibility::Restricted => Visibility::Restricted,
                        AstVisibility::Private => Visibility::Private,
                    },
                    span,
                    repr,
                });
                if has_derive_attr(&attrs, "Copy") {
                    add_clone_and_copy_for_def(ctx, id, span);
                }
            }
            Declaration::FunctionDefinition {
                attrs: decl_attrs,
                signature,
                body,
            } => {
                let ident_span = match &signature {
                    FunctionDefinitionSignature::C { declarator, .. } => {
                        declarator_ident_span(declarator).unwrap_or(parser_span)
                    }
                    FunctionDefinitionSignature::Rust(sig) => sig.name.1,
                };
                let (name, sig, attrs, no_mangle, visibility) = match signature {
                    FunctionDefinitionSignature::C {
                        declaration_specifiers,
                        declarator,
                    } => {
                        let is_static =
                            declaration_specifiers.iter().any(|spec| spec.0.is_static());
                        let transformed_specs = declaration_specifiers;
                        let base_const = has_const_qualifier_in_decl_specs(&transformed_specs);
                        let base = ctx.base_ty_of_decl(transformed_specs, parser_span);
                        let (name, sig) = ctx
                            .lower_function_signature(base, base_const, declarator)
                            .unwrap_or_else(|err| {
                                CrateSigCtx::<'_>::terminate_with_spanned_error(err)
                            });
                        if !no_main && module_path.is_empty() && name == "main" {
                            CrateSigCtx::<'_>::terminate_with_error(
                                ident_span,
                                "Main function with C ABI is not accepted in cargo projects. Use `fn main()` or `#![no_main]`.",
                            );
                        }
                        (name, sig, lower_generated_attrs(&decl_attrs), !is_static, Visibility::Public)
                    }
                    FunctionDefinitionSignature::Rust(sig) => {
                        let sig_attrs = lower_generated_attrs(&sig.attrs);
                        let mut attrs = lower_generated_attrs(&decl_attrs);
                        attrs.extend(sig_attrs);
                        let (name, lower_sig) = ctx.lower_rust_function_signature(sig.clone());
                        if !no_main
                            && module_path.is_empty()
                            && name == "main"
                            && !sig.params.is_empty()
                        {
                            CrateSigCtx::<'_>::terminate_with_error(
                                ident_span,
                                "Rust main function can not take arguments. Use `std::env::args()`.",
                            );
                        }
                        let no_mangle = has_word_attr(&attrs, "no_mangle");
                        let ast_vis = sig.visibility;
                        (name, lower_sig, attrs, no_mangle, match ast_vis {
                            AstVisibility::Public => Visibility::Public,
                            AstVisibility::Crate => Visibility::Crate,
                            AstVisibility::Restricted => Visibility::Restricted,
                            AstVisibility::Private => Visibility::Private,
                        })
                    }
                };

                let is_test = test && has_word_attr(&attrs, "test");
                let item_name = if is_test {
                    co2_test_symbol_name(module_path, &name)
                } else {
                    name.clone()
                };
                let body_param_names = sig
                    .inputs
                    .iter()
                    .enumerate()
                    .map(|(idx, input)| input.name.clone().unwrap_or_else(|| format!("arg{idx}")))
                    .collect::<Vec<_>>();
                let id = resolve_in_module(ctx, module_path, &name).0;
                let function_name = name.clone();
                let id = FnDef(id);
                hir_items.push(HirModuleItem::Function {
                    name: item_name,
                    id,
                    sig,
                    attrs,
                    no_mangle: no_mangle || is_test,
                    visibility,
                    span,
                    ident_span: ctx.co2_span_to_rustc(ident_span),
                });
                resolver = resolver.start_new_scope().with_owner(id.0);
                let param_names = body_param_names
                    .into_iter()
                    .map(|name| {
                        let id = resolver.add_local(name.clone());
                        (id, name)
                    })
                    .collect();
                let parse_body_start = Instant::now();
                let parsed_body = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    parse_compound_statement(
                        &body.0.tokens.0,
                        source_name,
                        source,
                        body.0.tokens.1,
                        resolver.clone(),
                    )
                }));
                if let Some(cb) = on_body_parsed {
                    cb(parse_body_start.elapsed());
                }

                let mir_owner = match parsed_body {
                    Ok(body) => MirOwnerInfo::Fn {
                        def: id,
                        function_name,
                        param_names,
                        resolver: resolver.clone(),
                        body,
                    },
                    Err(payload) => {
                        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                            MirOwnerInfo::FnBodyError {
                                def: id,
                                body_span: body.1,
                            }
                        } else {
                            std::panic::resume_unwind(payload);
                        }
                    }
                };

                ctx.mir_owners.insert(id.0, mir_owner);
            }
            Declaration::Declaration {
                attrs,
                declaration_specifiers,
                declarators,
            } => {
                let attrs = lower_generated_attrs(&attrs);
                let original_specs = declaration_specifiers.clone();
                let mut is_typedef = false;
                let mut is_extern = false;
                let mut is_static = false;
                let mut is_constexpr = false;
                let mut cleaned_specs = Vec::new();
                for (spec, sp) in declaration_specifiers {
                    match spec {
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Typedef,
                            _,
                        )) => {
                            is_typedef = true;
                        }
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Extern,
                            _,
                        )) => {
                            is_extern = true;
                        }
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Static,
                            _,
                        )) => {
                            is_static = true;
                        }
                        DeclarationSpecifier::StorageSpecifier((
                            StorageClassSpecifier::Constexpr,
                            _,
                        )) => {
                            is_constexpr = true;
                        }
                        _ => cleaned_specs.push((spec, sp)),
                    }
                }

                if is_constexpr && is_extern {
                    CrateSigCtx::<'_>::terminate_with_error(
                        constexpr_decl_span(&original_specs, parser_span),
                        "`constexpr` cannot be combined with `extern`",
                    );
                }

                let transformed_specs = cleaned_specs;
                let base_const = has_const_qualifier_in_decl_specs(&transformed_specs);
                let base = ctx.base_ty_of_decl(transformed_specs, parser_span);

                for init in declarators {
                    let InitDeclarator {
                        declarator,
                        initializer,
                        is_transparent_union,
                    } = init.0;
                    let declarator_for_checks = declarator.clone();

                    let (name, ty, array_len) =
                        ctx.lower_value_decl_ctype(base.clone(), base_const, declarator);

                    ctx.resolver
                        .borrow()
                        .global_locals
                        .borrow_mut()
                        .remove(&name);

                    if is_typedef {
                        let CTy::Ty(ty) = ty else {
                            ctx.resolver
                                .borrow_mut()
                                .unrepresentable_typedefs
                                .insert(name, ty);
                            continue;
                        };
                        let type_def = resolve_in_module(ctx, module_path, &name).0;
                        ctx.resolver
                            .borrow_mut()
                            .typedef_tys
                            .insert(type_def, ty.clone());
                        if is_transparent_union {
                            ctx.resolver.borrow_mut().mark_transparent_union(&ty);
                        }
                        hir_items.push(HirModuleItem::TypeDef {
                            name,
                            id: type_def,
                            attrs: attrs.clone(),
                            visibility: Visibility::Public,
                            span,
                            ty,
                        });
                        continue;
                    }
    
                    if let CTy::Ty(ty) = &ty {
                        let id = resolve_in_module(ctx, module_path, &name).0;
                        ctx.resolver
                            .borrow_mut()
                            .global_value_tys
                            .insert(id, ty.clone());
                    }
    
                    match ty {
                        CTy::Ty(ty) => {
                            let (id, _) = resolve_in_module(ctx, module_path, &name);
                            if is_constexpr {
                                ctx.resolver
                                    .borrow_mut()
                                    .validate_constexpr_decl(
                                        &original_specs,
                                        &declarator_for_checks.0,
                                        &CTy::Ty(ty.clone()),
                                        initializer.as_ref(),
                                    )
                                    .unwrap_or_else(|err| {
                                        CrateSigCtx::<'_>::terminate_with_spanned_error(err)
                                    });
                                if let Some((co2_ast::Initializer::Expr(expr), _span)) =
                                    initializer.clone()
                                {
                                    ctx.resolver
                                        .borrow_mut()
                                        .constexpr_def_exprs
                                        .insert(id, expr);
                                }
                            }
                            if is_extern {
                                foreign_items.push(ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else if is_constexpr && is_scalar_type(&ty) {
                                // For non-pointer constexpr, create an AnonConst DefId for the initializer
                                let rhs = ctx.allocate_def_id(id, &DefData::AnonConst);
                                if let Some(init) = initializer {
                                    ctx.mir_owners.insert(
                                        rhs,
                                        match array_len {
                                            Some(array_len) => MirOwnerInfo::StaticWithArrayLen {
                                                resolver: resolver.clone(),
                                                initializer: init,
                                                array_len,
                                            },
                                            None => MirOwnerInfo::Static {
                                                resolver: resolver.clone(),
                                                initializer: init,
                                            },
                                        },
                                    );
                                } else {
                                    ctx.mir_owners.insert(rhs, MirOwnerInfo::StaticZeroed);
                                }
                                // Register MirOwnerInfo for the Const item - needed for rustc's MIR query
                                ctx.mir_owners.insert(id, MirOwnerInfo::Const);
                                hir_items.push(HirModuleItem::Const {
                                    name,
                                    id,
                                    ty,
                                    rhs,
                                    attrs: attrs.clone(),
                                    visibility: Visibility::Public,
                                    span,
                                });
                            } else {
                                // Regular non-const static, or pointer constexpr (emit as static)
                                if let Some(init) = initializer {
                                    ctx.mir_owners.insert(
                                        id,
                                        match array_len {
                                            Some(array_len) => MirOwnerInfo::StaticWithArrayLen {
                                                resolver: resolver.clone(),
                                                initializer: init,
                                                array_len,
                                            },
                                            None => MirOwnerInfo::Static {
                                                resolver: resolver.clone(),
                                                initializer: init,
                                            },
                                        },
                                    );
                                } else {
                                    ctx.mir_owners.insert(id, MirOwnerInfo::StaticZeroed);
                                }
                                hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: false,
                                    no_mangle: !is_static,
                                    attrs: attrs.clone(),
                                    visibility: Visibility::Public,
                                    span,
                                });
                            }
                        }
                        CTy::UnsizedArray(elem_ty) => {
                            let (id, _) = resolve_in_module(ctx, module_path, &name);
                            if is_constexpr {
                                ctx.resolver
                                    .borrow_mut()
                                    .validate_constexpr_decl(
                                        &original_specs,
                                        &declarator_for_checks.0,
                                        &CTy::UnsizedArray(elem_ty.clone()),
                                        initializer.as_ref(),
                                    )
                                    .unwrap_or_else(|err| {
                                        CrateSigCtx::<'_>::terminate_with_spanned_error(err)
                                    });
                            }
                            if let Some(initializer) = initializer {
                                let len =
                                    infer_unsized_array_len(&initializer.0, &resolver, &elem_ty)
                                        .unwrap_or_else(|err| {
                                            CrateSigCtx::<'_>::terminate_with_spanned_error(err)
                                        });
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(len), span);
                                ctx.resolver
                                    .borrow_mut()
                                    .global_value_tys
                                    .insert(id, ty.clone());
                                ctx.mir_owners.insert(
                                    id,
                                    MirOwnerInfo::Static {
                                        resolver: resolver.clone(),
                                        initializer: initializer.clone(),
                                    },
                                );
                                hir_items.push(HirModuleItem::Static {
                                    name,
                                    id,
                                    ty,
                                    mutable: false,
                                    no_mangle: !is_static,
                                    attrs: attrs.clone(),
                                    visibility: Visibility::Public,
                                    span,
                                });
                            } else if is_extern {
                                let ty = HirTy::new_array(elem_ty, HirTyConst::Literal(0), span);
                                ctx.resolver
                                    .borrow_mut()
                                    .global_value_tys
                                    .insert(id, ty.clone());
                                foreign_items.push(ForeignModItem::ForeignStatic {
                                    name,
                                    id,
                                    ty,
                                    mutable: true,
                                    span,
                                });
                            } else {
                                CrateSigCtx::<'_>::terminate_with_error(
                                    parser_span,
                                    "static with unsized array type should have initializer",
                                );
                            }
                        }
                        CTy::Function(sig) => {
                            if is_constexpr {
                                CrateSigCtx::<'_>::terminate_with_error(
                                    constexpr_decl_span(&original_specs, parser_span),
                                    "`constexpr` object type must be scalar",
                                );
                            }
                            let def_id = resolve_in_module(ctx, module_path, &name).0;
                            let span = ctx.co2_span_to_rustc(init.1);
                            foreign_items.push(ForeignModItem::ForeignFunction {
                                name,
                                id: FnDef(def_id),
                                sig,
                                span,
                            });
                        }
                    }
                }
            }
            Declaration::BreakCo2 => {
                panic!("break co2!");
            }
            Declaration::PragmaPack { action } => {
                resolver.base.borrow_mut().apply_pack_action(&action);
            }
        }
    }

    for module in modules {
        let mut child_path = module_path.to_vec();
        child_path.push(module.name.clone());
        let items = lower_translation_unit_items(
            ctx,
            &module.tu,
            &module.children,
            &child_path,
            &module.source_name,
            module.source,
            foreign_mod,
            foreign_items,
            no_main,
            test,
            on_body_parsed,
        );
        let span = ctx.co2_span_to_rustc(module.decl_span);
        let mut module_item_attrs = lower_generated_attrs(&module.attrs);
        module_item_attrs.extend(lower_module_file_attrs_for_decl_item(&module.tu.attrs));
        hir_items.push(HirModuleItem::Module {
            name: module.name.clone(),
            id: module.def_id,
            module: HirModule {
                span,
                attrs: lower_generated_attrs(&module.tu.attrs),
                items,
            },
            attrs: module_item_attrs,
            visibility: Visibility::Public,
            span,
        });
    }
    hir_items
}

pub fn lower_crate_sig(
    ctx: HirStructureCtx<'_>,
    source_path: &Path,
    source_name: &str,
    src_static: &'static str,
    file_id: FileId,
    preprocessed: &Arc<PreprocessedSource>,
    file_ids: &mut HashMap<co2_ast::FileId, FileId, RandomState>,
    source_files: &mut HashMap<co2_ast::FileId, (String, Arc<str>), RandomState>,
    no_main: bool,
    test: bool,
    on_parse_done: Option<&mut dyn FnMut(Duration)>,
    mut on_body_parsed: Option<&mut dyn FnMut(Duration)>,
) -> (HirStructure, HashMap<DefId, MirOwnerInfo>, WellknownDefs) {
    let span = ctx.span_in_file(file_id, 0, 0);
    let deps = ctx.dependencies();
    deps.roots();

    let parse_start = Instant::now();
    let tu = co2_parser::parse_translation_unit_from_preprocessed(
        source_name,
        preprocessed.as_ref(),
        StatelessResolver::new(),
    )
    .expect("failed to parse co2 source")
    .0;
    if let Some(cb) = on_parse_done {
        cb(parse_start.elapsed());
    }

    let tu = deduplicate_tu_items(tu);
    let mut loaded_paths = HashSet::new();
    let loaded_modules = load_modules(
        &ctx,
        ctx.root_crate_def_id(),
        &root_module_dir(source_path),
        &tu.rust_mod_items,
        file_ids,
        source_files,
        &mut loaded_paths,
    );
    co2_ast::set_source_map(Arc::new(SourceMapSnapshot {
        files: Arc::new(source_files.clone()),
    }));

    let foreign_mod_kind = if test {
        DefData::TypeNs("__co2_foreign_mod".to_owned())
    } else {
        DefData::ForeignMod
    };
    let foreign_mod = ctx.allocate_def_id(ctx.root_crate_def_id(), &foreign_mod_kind);
    let mut foreign_items = Vec::new();

    let ctx = &*Box::leak(Box::new(ctx));
    let ctx_static: &'static HirStructureCtx<'static> = unsafe {
        std::mem::transmute::<&HirStructureCtx<'_>, &'static HirStructureCtx<'static>>(ctx)
    };
    let mut resolver = Resolver::new(ctx_static, deps, &tu, foreign_mod, test);

    let loaded_modules = match loaded_modules {
        Ok(l) => l,
        Err(e) => co2_ast::emit_errors_and_terminate(vec![Rich::custom(
            e.name.1,
            format!("file not found for module `{}`", e.name.0),
        )]),
    };

    for module in &loaded_modules {
        let module_path = vec![module.name.clone()];
        resolver.insert_module_data(
            &[],
            &module.name,
            build_module_data_tree(ctx, module, foreign_mod, &module_path, test),
        );
    }
    resolver.import_use_items(&[], &tu);
    import_module_use_items(&mut resolver, &[], &loaded_modules);
    resolver.rebuild_method_receivers();
    let file_ids = Arc::new(file_ids.clone());

    let mut ctx = CrateSigCtx {
        clone_trait: resolver.resolve("core::clone::Clone").unwrap().0,
        copy_trait: resolver.resolve("core::marker::Copy").unwrap().0,
        clone_trait_fn: resolver.resolve("core::clone::Clone::clone").unwrap().0,
        resolver: Rc::new(RefCell::new(LocalResolverBase {
            resolver,
            local_counter: 0,
            fake_defs_counter: 0,
            array_len_const_counter: 0,
            pending_typedefs: vec![],
            pending_static: vec![],
            array_len_consts: HashMap::new(),
            array_len_const_exprs: HashMap::new(),
            hir_ctx: unsafe {
                std::mem::transmute::<&HirStructureCtx<'_>, &'static HirStructureCtx<'static>>(ctx)
            },
            file_id,
            preprocessed: preprocessed.clone(),
            file_ids: file_ids.clone(),
            struct_manager: StructManager::default(),
            unrepresentable_typedefs: HashMap::new(),
            typedef_tys: HashMap::new(),
            transparent_unions: HashSet::new(),
            global_value_tys: HashMap::new(),
            global_struct_tags: Rc::new(RefCell::new(StructAndEnumData::default())),
            global_locals: Rc::new(RefCell::new(im::HashMap::new())),
            enum_const_values: HashMap::new(),
            constexpr_def_exprs: HashMap::new(),
            constexpr_local_exprs: HashMap::new(),
        })),
        hir_ctx: ctx,
        file_ids,
        mir_owners: HashMap::new(),
        hir_items: vec![],
    };

    {
        let adt = ctx.resolve("core::ffi::VaList").unwrap().0;
        let ty = HirTy::adt(
            adt,
            vec![HirGenericArg::Lifetime(HirLifetime::Static)],
            span,
        );
        for name in ["__builtin_va_list", "__gnuc_va_list"] {
            let Ok((id, _)) = ctx.resolve(name) else {
                continue;
            };
            ctx.resolver.borrow_mut().typedef_tys.insert(id, ty.clone());
            ctx.hir_items.push(HirModuleItem::TypeDef {
                name: name.to_owned(),
                id,
                attrs: Vec::new(),
                visibility: Visibility::Public,
                span,
                ty: ty.clone(),
            });
        }
    }

    validate_all_attrs(&tu, &loaded_modules);

    let root_items = lower_translation_unit_items(
        &mut ctx,
        &tu,
        &loaded_modules,
        &[],
        source_name,
        src_static,
        foreign_mod,
        &mut foreign_items,
        no_main,
        test,
        &mut on_body_parsed,
    );
    ctx.hir_items.extend(root_items);

    let pending_typedefs = std::mem::take(&mut ctx.resolver.borrow_mut().pending_typedefs);
    for (id, name, specifiers, declarator, parser_span, is_transparent_union) in pending_typedefs {
        let span = ctx.co2_span_to_rustc(parser_span);
        let base_const = has_const_qualifier_in_decl_specs(&specifiers);
        let ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let (_, ty, _) = ctx.lower_value_decl_ctype(ty, base_const, (declarator, parser_span));
        let CTy::Ty(ty) = ty else {
            CrateSigCtx::<'_>::terminate_with_error(
                parser_span,
                "typedef did not lower to a first-class type",
            );
        };
        ctx.resolver.borrow_mut().typedef_tys.insert(id, ty.clone());
        if is_transparent_union {
            ctx.resolver.borrow_mut().mark_transparent_union(&ty);
        }
        ctx.hir_items.push(HirModuleItem::TypeDef {
            name,
            id,
            ty,
            attrs: Vec::new(),
            visibility: Visibility::Public,
            span,
        });
    }

    let pending_static = std::mem::take(&mut ctx.resolver.borrow_mut().pending_static);
    for (id, name, specifiers, declarator, parser_span) in pending_static {
        let span = ctx.co2_span_to_rustc(parser_span);
        let original_specs = specifiers.clone();
        let is_constexpr = specifiers.iter().any(|spec| spec.0.is_constexpr());
        let base_const = has_const_qualifier_in_decl_specs(&specifiers);
        let base_ty = ctx.base_ty_of_decl(specifiers, parser_span);
        let resolver = LocalResolver::new(ctx.resolver.clone());
        let declarator_for_checks = declarator.declarator.clone();
        let (_, ty, _) = ctx.lower_value_decl_ctype(base_ty, base_const, declarator.declarator);
        if let CTy::Ty(ty) = &ty {
            ctx.resolver
                .borrow_mut()
                .global_value_tys
                .insert(id, ty.clone());
        }
        match ty {
            CTy::Ty(ty) => {
                if is_constexpr {
                    ctx.resolver
                        .borrow_mut()
                        .validate_constexpr_decl(
                            &original_specs,
                            &declarator_for_checks.0,
                            &CTy::Ty(ty.clone()),
                            declarator.initializer.as_ref(),
                        )
                        .unwrap_or_else(|err| CrateSigCtx::<'_>::terminate_with_spanned_error(err));
                    if let Some((co2_ast::Initializer::Expr(expr), _span)) =
                        declarator.initializer.clone()
                    {
                        ctx.resolver
                            .borrow_mut()
                            .constexpr_def_exprs
                            .insert(id, expr);
                    }
                }
                ctx.hir_items.push(HirModuleItem::Static {
                    name,
                    id,
                    ty,
                    span,
                    mutable: !is_constexpr,
                    no_mangle: false,
                    attrs: Vec::new(),
                    visibility: Visibility::Public,
                });
                if let Some(initializer) = declarator.initializer {
                    ctx.mir_owners.insert(
                        id,
                        MirOwnerInfo::Static {
                            resolver: resolver.clone(),
                            initializer,
                        },
                    );
                } else {
                    ctx.mir_owners.insert(id, MirOwnerInfo::StaticZeroed);
                }
            }
            CTy::UnsizedArray(elem_ty) => {
                if is_constexpr {
                    ctx.resolver
                        .borrow_mut()
                        .validate_constexpr_decl(
                            &original_specs,
                            &declarator_for_checks.0,
                            &CTy::UnsizedArray(elem_ty.clone()),
                            declarator.initializer.as_ref(),
                        )
                        .unwrap_or_else(|err| CrateSigCtx::<'_>::terminate_with_spanned_error(err));
                }
                let initializer = if let Some((initializer, init_span)) = declarator.initializer {
                    (initializer, init_span)
                } else {
                    CrateSigCtx::<'_>::terminate_with_error(
                        parser_span,
                        "local static with unsized array type must have an initializer",
                    );
                };
                let len = infer_unsized_array_len(&initializer.0, &resolver, &elem_ty)
                    .unwrap_or_else(|err| CrateSigCtx::<'_>::terminate_with_spanned_error(err));
                let sized_ty = HirTy::new_array(elem_ty, HirTyConst::Literal(len), span);
                ctx.resolver
                    .borrow_mut()
                    .global_value_tys
                    .insert(id, sized_ty.clone());
                ctx.hir_items.push(HirModuleItem::Static {
                    name,
                    id,
                    ty: sized_ty,
                    span,
                    mutable: !is_constexpr,
                    no_mangle: false,
                    attrs: Vec::new(),
                    visibility: Visibility::Public,
                });
                ctx.mir_owners.insert(
                    id,
                    MirOwnerInfo::Static {
                        resolver: resolver.clone(),
                        initializer,
                    },
                );
            }
            CTy::Function(_) => {
                CrateSigCtx::<'_>::terminate_with_error(
                    parser_span,
                    "static did not lower to a first-class type",
                );
            }
        }
    }

    let structs = ctx.resolver.borrow_mut().emit_structs().collect::<Vec<_>>();
    for StructData {
        def_id: def,
        name,
        tag_name: _,
        kind,
        emitted_fields: fields,
        logical_fields: _,
        span,
        pack_align,
        skip_emit,
    } in structs
    {
        if skip_emit {
            continue;
        }
        let Some(fields) = fields else {
            let foreign_name = format!("{name}__foreign");
            let foreign_def = ctx.allocate_def_id(foreign_mod, &DefData::TypeNs(foreign_name));
            foreign_items.push(ForeignModItem::ForeignType {
                name: format!("{name}__foreign"),
                id: foreign_def,
                span,
            });
            let typedef_hir_ty = HirTy::adt(foreign_def, vec![], span);
            // Record the mapping so that uses of this incomplete struct type (via
            // StructOrUnion specifier) can resolve to the ForeignDef rather than
            // the TyAlias, which would ICE when rustc calls `adt_def` on it.
            ctx.resolver
                .borrow_mut()
                .typedef_tys
                .insert(def, typedef_hir_ty.clone());
            ctx.hir_items.push(HirModuleItem::TypeDef {
                name,
                id: def,
                ty: typedef_hir_ty,
                attrs: Vec::new(),
                visibility: Visibility::Public,
                span,
            });
            continue;
        };
        let kind = match kind {
            StructOrUnionKind::Struct => HirAdtKind::Struct { fields },
            StructOrUnionKind::Union => HirAdtKind::Union { fields },
        };
        let repr = match pack_align {
            Some(n) => AdtRepr::CPacked(n),
            None => AdtRepr::C,
        };

        ctx.hir_items.push(HirModuleItem::Adt {
            name,
            id: AdtDef(def),
            kind,
            span,
            repr,
            visibility: Visibility::Public,
        });

        add_clone_and_copy_for_def(&mut ctx, def, span);
    }

    ctx.hir_items.push(HirModuleItem::ForeignMod {
        id: foreign_mod,
        items: foreign_items,
    });

    let enums = ctx.resolver.borrow_mut().emit_enums().collect::<Vec<_>>();
    for PendingEnum {
        name,
        def_id,
        mir_info,
    } in enums
    {
        ctx.hir_items
            .push(rustc_public_generative::HirModuleItem::Static {
                name: name.clone(),
                id: def_id,
                ty: HirTy::signed_ty(IntTy::I32, span),
                mutable: false,
                no_mangle: false,
                attrs: Vec::new(),
                visibility: Visibility::Public,
                span,
            });
        ctx.mir_owners.insert(def_id, mir_info);
    }

    let mut array_len_consts = ctx
        .resolver
        .borrow()
        .array_len_consts
        .values()
        .filter(|registered| !expr_contains_local(&registered.expr.0))
        .cloned()
        .collect::<Vec<_>>();
    array_len_consts.sort_by_key(|registered| registered.id);
    for registered in array_len_consts {
        let span = ctx.co2_span_to_rustc(registered.span);
        let value = ctx
            .resolver
            .borrow_mut()
            .eval_array_len_expr(&registered.expr)
            .unwrap_or_else(|err| CrateSigCtx::<'_>::terminate_with_spanned_error(err));
        let expr = (
            Expression::Constant(Constant::Int(value as i128, IntegerSuffix::None)),
            registered.span,
        );
        ctx.hir_items.push(HirModuleItem::Const {
            name: registered.name,
            id: registered.def_id,
            ty: HirTy::usize_ty(span),
            rhs: registered.rhs,
            attrs: Vec::new(),
            visibility: Visibility::Public,
            span,
        });
        ctx.mir_owners.insert(
            registered.rhs,
            MirOwnerInfo::Static {
                resolver: registered.resolver,
                initializer: (Initializer::Expr(expr), registered.span),
            },
        );
        ctx.mir_owners
            .insert(registered.def_id, MirOwnerInfo::Const);
    }

    let resolve_inherent_method = |receiver_ty: Ty, method: &str| {
        FnDef(
            ctx.resolver
                .borrow_mut()
                .resolver
                .resolve_inherent_method_for_sig(receiver_ty, method)
                .unwrap()
                .0,
        )
    };

    let defs = WellknownDefs {
        maybe_uninit: AdtDef(ctx.resolve("core::mem::MaybeUninit").unwrap().0),
        maybe_uninit_uninit: FnDef(
            ctx.resolve("core::mem::MaybeUninit::<T>::uninit")
                .unwrap()
                .0,
        ),
        valist: AdtDef(ctx.resolve("core::ffi::VaList").unwrap().0),
        valist_fn_arg: FnDef(ctx.resolve("core::ffi::VaList::<'f>::next_arg").unwrap().0),
        clone: FnDef(ctx.resolve("core::clone::Clone::clone").unwrap().0),
        zeroed: FnDef(ctx.resolve("core::mem::zeroed").unwrap().0),
        transmute: FnDef(ctx.resolve("core::intrinsics::transmute").unwrap().0),
        transmute_copy: FnDef(ctx.resolve("core::mem::transmute_copy").unwrap().0),
        offset_mut: resolve_inherent_method(
            Ty::new_ptr(
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U8)),
                Mutability::Mut,
            ),
            "offset",
        ),
        offset_const: resolve_inherent_method(
            Ty::new_ptr(
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U8)),
                Mutability::Not,
            ),
            "offset",
        ),
        offset_from: resolve_inherent_method(
            Ty::new_ptr(
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U8)),
                Mutability::Not,
            ),
            "offset_from",
        ),
    };
    (
        HirStructure {
            root: HirModule {
                span,
                attrs: lower_module_file_attrs_for_decl_item(&tu.attrs),
                items: ctx.hir_items,
            },
            no_main,
        },
        ctx.mir_owners,
        defs,
    )
}

fn add_clone_and_copy_for_def(
    ctx: &mut CrateSigCtx<'_>,
    def: DefId,
    span: rustc_public_generative::rustc_public::ty::Span,
) {
    let self_ty_hir = HirTy::adt(def, vec![], span);

    let root_crate = ctx.root_crate_def_id();
    let clone_impl_def = ctx.allocate_def_id(root_crate, &DefData::Impl);
    let clone_method_def =
        ctx.allocate_def_id(clone_impl_def, &DefData::ValueNs("clone".to_owned()));
    let clone_self_lifetime =
        ctx.allocate_def_id(clone_method_def, &DefData::LifetimeNs("a".to_owned()));
    let clone_sig = FunctionSignature {
        lifetimes: vec![clone_self_lifetime],
        inputs: Vec::new(),
        output: self_ty_hir.clone(),
        abi: FunctionAbi::Rust,
        is_unsafe: false,
        c_variadic: false,
    };
    ctx.hir_items.push(HirModuleItem::Impl {
        id: clone_impl_def,
        self_ty: self_ty_hir.clone(),
        trait_def: Some(ctx.clone_trait),
        items: vec![HirImplItem {
            name: "clone".to_owned(),
            id: clone_method_def,
            kind: HirImplItemKind::Fn {
                sig: clone_sig,
                self_kind: HirSelfKind::RefImm(HirLifetime::Param(clone_self_lifetime)),
                trait_item_def_id: Some(ctx.clone_trait_fn),
            },
            span,
        }],
        span,
    });
    ctx.mir_owners
        .insert(clone_method_def, MirOwnerInfo::CloneMethod(AdtDef(def)));

    let copy_impl_def = ctx.allocate_def_id(root_crate, &DefData::Impl);
    ctx.hir_items.push(HirModuleItem::Impl {
        id: copy_impl_def,
        self_ty: self_ty_hir.clone(),
        trait_def: Some(ctx.copy_trait),
        items: Vec::new(),
        span,
    });
}

// TODO: this function is AI garbage and is duplicate logic from what is in co2_hir
fn infer_unsized_array_len(
    initializer: &co2_ast::Initializer<LocalResolver>,
    resolver: &LocalResolver,
    elem_ty: &HirTy,
) -> Result<usize, (co2_ast::Span, String)> {
    match initializer {
        co2_ast::Initializer::Expr((
            co2_ast::Expression::Constant(co2_ast::Constant::String(s)),
            _,
        )) => Ok(s.nul_terminated_len()),
        co2_ast::Initializer::List(items) => {
            let slots_per_elem = flattened_scalar_slots(elem_ty, resolver)?;
            let mut next_index = 0usize;
            let mut max_len = 0usize;
            let mut used_slots_in_current = 0usize;
            for (item, _) in items {
                let index = match &item.designators {
                    None => next_index,
                    Some(designators) => match designators.first() {
                        None => next_index,
                        Some((first, _)) => match first {
                            Designator::Subscript(expr) => {
                                let mut base = resolver.base.borrow_mut();
                                let value = base.eval_const_expr(expr)?;
                                usize::try_from(value).map_err(|_| {
                                        (expr.1, format!("array designator index must be non-negative, got {value}"))
                                    })?
                            }
                            Designator::Field(_) => {
                                return Err((
                                        item.initializer.1,
                                        "field designator is invalid for unsized array length inference"
                                            .to_owned(),
                                    ));
                            }
                            Designator::Range(_, _) => {
                                return Err((
                                    item.initializer.1,
                                    "unsupported GNU range designator".to_owned(),
                                ));
                            }
                        },
                    },
                };
                if index != next_index {
                    used_slots_in_current = 0;
                }
                let consumed_slots =
                    consumed_initializer_slots(&item.initializer.0, elem_ty, resolver)?;
                let element_advance = if consumed_slots == 0 {
                    1
                } else {
                    consumed_slots
                };
                let total_slots = used_slots_in_current + element_advance;
                let fully_covered = total_slots.div_ceil(slots_per_elem);
                max_len = max_len.max(index + fully_covered);
                next_index = index + total_slots / slots_per_elem;
                used_slots_in_current = total_slots % slots_per_elem;
            }
            Ok(max_len)
        }
        co2_ast::Initializer::Expr((_, span)) => Err((
            *span,
            "static with unsized array type should have list or string initializer".to_owned(),
        )),
    }
}

fn consumed_initializer_slots(
    initializer: &co2_ast::Initializer<LocalResolver>,
    target_ty: &HirTy,
    resolver: &LocalResolver,
) -> Result<usize, (co2_ast::Span, String)> {
    match initializer {
        co2_ast::Initializer::Expr(_) => Ok(1),
        co2_ast::Initializer::List(_) => flattened_scalar_slots(target_ty, resolver),
    }
}

fn flattened_scalar_slots(
    ty: &HirTy,
    resolver: &LocalResolver,
) -> Result<usize, (co2_ast::Span, String)> {
    match &ty.kind {
        rustc_public_generative::HirTyKind::Array(HirTyConst::Literal(len), inner) => {
            Ok(len * flattened_scalar_slots(inner, resolver)?)
        }
        rustc_public_generative::HirTyKind::Adt(def, _) => {
            let base = resolver.base.borrow();
            if let Some((kind, fields)) = base.adt_layout_info(*def) {
                match kind {
                    StructOrUnionKind::Struct => fields.iter().try_fold(0usize, |acc, field| {
                        Ok(acc + flattened_scalar_slots(field, resolver)?)
                    }),
                    StructOrUnionKind::Union => fields
                        .first()
                        .map_or(Ok(1), |field| flattened_scalar_slots(field, resolver)),
                }
            } else if let Some(aliased) = base.typedef_tys.get(def) {
                flattened_scalar_slots(aliased, resolver)
            } else {
                // Unknown ADT (e.g. MaybeUninit<fn(...)> for function pointers, or
                // other Rust stdlib types): opaque to our layout model, counts as one
                // initializer slot just like any scalar.
                Ok(1)
            }
        }
        // Tuple (including unit `()`) and any other opaque types: treat as one slot.
        _ => Ok(1),
    }
}
