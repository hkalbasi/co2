use std::collections::{HashMap, HashSet};
use std::fmt;

use co2_ast::{
    Declaration, Declarator, StatelessResolver, TranslationUnit, TypeQueryResult,
    co2_test_symbol_name,
};

#[derive(Debug, Clone)]
pub struct ResolveError; // TODO: add reason of failure.

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to resolve path")
    }
}
use rustc_public_generative::{
    DefData, DependencyChildKind, DependencyInfo, HirStructureCtx,
    rustc_public::{
        DefId,
        ty::{RigidTy, Ty, TyKind, UintTy},
    },
};

#[derive(Debug, Clone)]
pub(crate) enum ModuleData {
    Unexpanded(DefId),
    Expanded(Box<ModuleContent>),
}

impl Default for ModuleData {
    fn default() -> Self {
        Self::Expanded(Box::default())
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ModuleContent {
    pub id: Option<(DefId, TypeQueryResult)>,
    pub items: HashMap<String, ModuleData>,
}

#[derive(Debug, Clone)]
struct ScopedTrait {
    name: String,
    def_id: DefId,
    path: String,
}

fn extract_decl_name(decl: &Declarator<StatelessResolver>) -> Option<String> {
    match decl {
        Declarator::Abstract => None,
        Declarator::Identifier((name, _)) => Some(name.clone()),
        Declarator::FunctionDeclarator {
            declarator,
            param_list: _,
        }
        | Declarator::PointerDeclarator {
            declarator,
            qualifiers: _,
        }
        | Declarator::ArrayDeclarator {
            declarator,
            subscription: _,
        } => extract_decl_name(&declarator.0),
    }
}

fn type_query_result(kind: &DependencyChildKind) -> TypeQueryResult {
    match kind {
        DependencyChildKind::Function
        | DependencyChildKind::Const
        | DependencyChildKind::Static => TypeQueryResult::Expr,
        DependencyChildKind::Struct
        | DependencyChildKind::Enum
        | DependencyChildKind::Union
        | DependencyChildKind::Trait
        | DependencyChildKind::Module
        | DependencyChildKind::TypeAlias
        | DependencyChildKind::Other => TypeQueryResult::Type,
    }
}

impl ModuleData {
    fn as_content_mut(&mut self) -> &mut ModuleContent {
        match self {
            Self::Expanded(c) => c,
            Self::Unexpanded(_) => panic!("ModuleData not expanded"),
        }
    }

    fn fill_impl_children(
        content: &mut ModuleContent,
        def_id: DefId,
        dep_info: &DependencyInfo<'_>,
    ) {
        let impls = dep_info.impls(def_id);
        for impl_fn in impls {
            content
                .items
                .entry(impl_fn.name.clone())
                .or_insert_with(|| {
                    ModuleData::Expanded(Box::new(ModuleContent {
                        id: Some((impl_fn.def_id, TypeQueryResult::Expr)),
                        ..Default::default()
                    }))
                });
        }
    }

    fn ensure_expanded(&mut self, dep_info: &DependencyInfo<'_>) {
        let Some(def_id) = (match self {
            Self::Unexpanded(def_id) => Some(*def_id),
            Self::Expanded(_) => None,
        }) else {
            return;
        };
        let mut content = ModuleContent::default();
        let children = dep_info.children(def_id);
        for child in children {
            let mut child_content = ModuleContent {
                id: Some((child.def_id, type_query_result(&child.kind))),
                ..Default::default()
            };
            let child_data = match child.kind {
                DependencyChildKind::Module | DependencyChildKind::Trait => {
                    ModuleData::Unexpanded(child.def_id)
                }
                _ => {
                    // Pre-populate inherent impl methods for types (structs, enums, unions)
                    Self::fill_impl_children(&mut child_content, child.def_id, dep_info);
                    ModuleData::Expanded(Box::new(child_content))
                }
            };
            match content.items.entry(child.name) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(child_data);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    // Prefer public items over private (pub(crate)) items with the same name.
                    // This prevents internal sub-modules (e.g. `pub(crate) mod read;`) from
                    // shadowing public items (e.g. `pub fn read()`) in dependency crates.
                    if child.pub_vis {
                        entry.insert(child_data);
                    }
                }
            }
        }
        // Also add inherent impl methods for this def_id (for traits, modules, etc.)
        Self::fill_impl_children(&mut content, def_id, dep_info);
        content.id = Some((def_id, TypeQueryResult::Type));
        *self = Self::Expanded(Box::new(content));
    }

    fn insert_path<'a>(
        &mut self,
        mut path: impl Iterator<Item = &'a str>,
        def: Option<(DefId, TypeQueryResult)>,
    ) {
        let content = self.as_content_mut();
        let Some(seg1) = path.next() else {
            content.id = def;
            return;
        };
        let part = content.items.entry(seg1.to_owned()).or_default();
        part.insert_path(path, def);
    }

    fn resolve_path<'a>(
        &'a mut self,
        path: impl Iterator<Item = &'a str>,
        dep_info: &'a DependencyInfo<'_>,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        let parts = path.collect::<Vec<_>>();
        self.resolve_path_inner(&parts, dep_info)
    }

    fn resolve_path_inner(
        &mut self,
        path: &[&str],
        dep_info: &DependencyInfo<'_>,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        self.ensure_expanded(dep_info);
        let content = self.as_content_mut();
        if path.is_empty() {
            return content.id;
        }
        // Skip generic parameter segments like <T>, <'f>, <impl str>
        if path[0].starts_with('<') {
            return self.resolve_path_inner(&path[1..], dep_info);
        }
        content
            .items
            .get_mut(path[0])?
            .resolve_path_inner(&path[1..], dep_info)
    }

    pub(crate) fn insert_alias(&mut self, alias: &str, item: ModuleData) {
        self.as_content_mut().items.insert(alias.to_owned(), item);
    }

    pub(crate) fn forward_pass_parsed_module(
        ctx: &HirStructureCtx<'_>,
        ast: &TranslationUnit<StatelessResolver>,
        parent: DefId,
        foreign_mod: DefId,
        module_path: &[String],
        include_builtin_va_list: bool,
        test: bool,
    ) -> Self {
        let mut this = ModuleContent::default();
        if include_builtin_va_list {
            for name in ["__builtin_va_list", "__gnuc_va_list"] {
                let def_id = ctx.allocate_def_id(parent, &DefData::TypeNs(name.to_owned()));
                this.items.insert(
                    name.to_owned(),
                    ModuleData::Expanded(Box::new(ModuleContent {
                        id: Some((def_id, TypeQueryResult::Type)),
                        ..Default::default()
                    })),
                );
            }
        }
        for (item, _) in &ast.items {
            match item {
                Declaration::FunctionDefinition { signature, .. } => {
                    let Some(decl) = signature.ident() else {
                        continue;
                    };
                    let def_name = if test
                        && matches!(
                            signature,
                            co2_ast::FunctionDefinitionSignature::Rust(sig)
                                if sig.attrs.iter().any(|(attr, _)| attr.is_word("test"))
                        ) {
                        co2_test_symbol_name(module_path, decl.as_str())
                    } else {
                        decl.clone()
                    };
                    let def_id = ctx.allocate_def_id(parent, &DefData::ValueNs(def_name));
                    this.items.insert(
                        decl.clone(),
                        ModuleData::Expanded(Box::new(ModuleContent {
                            id: Some((def_id, TypeQueryResult::Expr)),
                            ..Default::default()
                        })),
                    );
                }
                Declaration::RustTypeAlias { ident, .. }
                | Declaration::RustStruct { ident, .. } => {
                    let name = ident.0.as_str();
                    let def_id = ctx.allocate_def_id(parent, &DefData::TypeNs(name.to_owned()));
                    this.items.insert(
                        name.to_owned(),
                        ModuleData::Expanded(Box::new(ModuleContent {
                            id: Some((def_id, TypeQueryResult::Type)),
                            ..Default::default()
                        })),
                    );
                }
                Declaration::Declaration {
                    attrs: _,
                    declaration_specifiers,
                    declarators,
                } => {
                    let is_typedef = declaration_specifiers.iter().any(|x| x.0.is_typedef());
                    let is_extern = declaration_specifiers.iter().any(|x| x.0.is_extern());
                    if is_typedef {
                        for decl in declarators {
                            let decl = &decl.0.declarator.0;
                            if decl.is_function() {
                                continue;
                            }
                            let Some(name) = extract_decl_name(decl) else {
                                continue;
                            };
                            if this.items.contains_key(&name) {
                                continue;
                            }
                            let def_id =
                                ctx.allocate_def_id(parent, &DefData::TypeNs(name.clone()));
                            this.items.insert(
                                name,
                                ModuleData::Expanded(Box::new(ModuleContent {
                                    id: Some((def_id, TypeQueryResult::Type)),
                                    ..Default::default()
                                })),
                            );
                        }
                    } else {
                        for decl in declarators {
                            let decl = &decl.0.declarator.0;
                            let Some(name) = extract_decl_name(decl) else {
                                continue;
                            };
                            if this.items.contains_key(&name) {
                                continue;
                            }
                            let parent = if decl.is_function() || is_extern {
                                foreign_mod
                            } else {
                                parent
                            };
                            let def_id =
                                ctx.allocate_def_id(parent, &DefData::ValueNs(name.clone()));
                            this.items.insert(
                                name,
                                ModuleData::Expanded(Box::new(ModuleContent {
                                    id: Some((def_id, TypeQueryResult::Expr)),
                                    ..Default::default()
                                })),
                            );
                        }
                    }
                }
                Declaration::PragmaPack { .. } | Declaration::BreakCo2 => {}
            }
        }
        ModuleData::Expanded(Box::new(this))
    }
}

fn anchored_module_prefix<'a>(
    module_path: &'a [String],
    parts: &'a [&'a str],
) -> Option<(Vec<&'a str>, &'a [&'a str])> {
    let first = parts.first().copied()?;
    match first {
        "crate" => Some((Vec::new(), &parts[1..])),
        "super" => {
            let mut supers = 0usize;
            while supers < parts.len() && parts[supers] == "super" {
                supers += 1;
            }
            if supers > module_path.len() {
                return None;
            }
            Some((
                module_path[..module_path.len() - supers]
                    .iter()
                    .map(String::as_str)
                    .collect(),
                &parts[supers..],
            ))
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct Resolver {
    method_receivers: HashMap<DefId, ModuleData>,
    dependencies: HashMap<String, ModuleData>,
    current: ModuleData,
    scoped_traits: Vec<ScopedTrait>,
    hir_ctx: &'static HirStructureCtx<'static>,
}

fn normalize_crate_name(name: &mut &str) {
    if *name == "std" || *name == "core" || *name == "alloc" {
        *name = "std_core";
    }
}

impl Resolver {
    pub(crate) fn new(
        ctx: &'static HirStructureCtx<'static>,
        deps: DependencyInfo<'_>,
        p: &TranslationUnit<StatelessResolver>,
        foreign_mod: DefId,
        test: bool,
    ) -> Self {
        let hir_ctx = ctx;
        let mut this = Self {
            method_receivers: HashMap::new(),
            dependencies: HashMap::new(),
            current: ModuleData::default(),
            scoped_traits: Vec::new(),
            hir_ctx,
        };

        for (krate, root_def_id) in deps.roots() {
            this.dependencies
                .insert(krate.name, ModuleData::Unexpanded(root_def_id));
        }

        this.current = ModuleData::forward_pass_parsed_module(
            ctx,
            p,
            ctx.root_crate_def_id(),
            foreign_mod,
            &[],
            true,
            test,
        );

        let builtin_mappings: &[(Ty, &str, &[&str])] = &[
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U16)),
                "swap_bytes",
                &["__builtin_bswap16"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U32)),
                "swap_bytes",
                &["__builtin_bswap32"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U64)),
                "swap_bytes",
                &["__builtin_bswap64"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U32)),
                "leading_zeros",
                &["__builtin_clz"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U64)),
                "leading_zeros",
                &["__builtin_clzll"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U32)),
                "trailing_zeros",
                &["__builtin_ctz"],
            ),
            (
                Ty::from_rigid_kind(RigidTy::Uint(UintTy::U64)),
                "trailing_zeros",
                &["__builtin_ctzll"],
            ),
        ];
        for &(receiver_ty, method, aliases) in builtin_mappings {
            if let Some(def) = this.resolve_inherent_method_for_sig(receiver_ty, method) {
                for &alias in aliases {
                    this.current.insert_path([alias].into_iter(), Some(def));
                }
            }
        }

        // Populate prelude traits (auto-imported like Clone, Copy, etc.)
        // Try resolving them from known crate paths
        let prelude_trait_names: &[(&str, &str)] = &[
            ("std", "borrow::ToOwned"),
            ("core", "clone::Clone"),
            ("core", "marker::Copy"),
            ("core", "convert::Into"),
            ("core", "convert::From"),
            ("core", "iter::IntoIterator"),
            ("core", "iter::Iterator"),
            ("core", "fmt::ToString"),
            ("core", "convert::AsRef"),
            ("core", "convert::AsMut"),
            ("core", "default::Default"),
            ("core", "cmp::PartialEq"),
            ("core", "cmp::Eq"),
            ("core", "cmp::PartialOrd"),
            ("core", "cmp::Ord"),
            ("core", "ops::Drop"),
            ("core", "ops::Fn"),
            ("core", "ops::FnMut"),
            ("core", "ops::FnOnce"),
            ("core", "marker::Send"),
            ("core", "marker::Sync"),
            ("core", "marker::Sized"),
            ("core", "marker::Unpin"),
            ("core", "borrow::Borrow"),
            ("core", "borrow::BorrowMut"),
            ("core", "iter::DoubleEndedIterator"),
            ("core", "iter::ExactSizeIterator"),
            ("core", "iter::Extend"),
            ("core", "iter::FromIterator"),
        ];
        for &(crate_name, rest) in prelude_trait_names {
            if let Ok((def_id, _)) = this.resolve(&format!("{crate_name}::{rest}")) {
                let name = rest.split("::").last().unwrap_or(rest);
                this.scoped_traits.push(ScopedTrait {
                    name: name.to_owned(),
                    def_id,
                    path: format!("{crate_name}::{rest}"),
                });
            }
        }

        this
    }

    fn dep_info(&self) -> DependencyInfo<'static> {
        DependencyInfo {
            tcx: self.hir_ctx.tcx,
        }
    }

    fn module_mut<'a>(&'a mut self, path: &[String]) -> &'a mut ModuleData {
        let mut module = &mut self.current;
        for segment in path {
            module = match module {
                ModuleData::Expanded(c) => c.items.entry(segment.clone()).or_default(),
                ModuleData::Unexpanded(_) => panic!("current module must be expanded"),
            };
        }
        module
    }

    fn resolve_module_node_relative<'a>(
        &mut self,
        module_path: &[String],
        path: impl IntoIterator<Item = &'a str>,
    ) -> Option<ModuleData> {
        fn descend(
            node: &mut ModuleData,
            parts: &[&str],
            info: &DependencyInfo<'_>,
        ) -> Option<ModuleData> {
            node.ensure_expanded(info);
            if parts.is_empty() {
                return Some(node.clone());
            }
            let child = node.as_content_mut().items.get_mut(parts[0])?;
            descend(child, &parts[1..], info)
        }

        let parts: Vec<&str> = path.into_iter().collect();
        let info = self.dep_info();

        if let Some((prefix, rest)) = anchored_module_prefix(module_path, &parts) {
            let mut current = &mut self.current;
            for segment in &prefix {
                current = current.as_content_mut().items.get_mut(*segment)?;
            }
            return descend(current, rest, &info);
        }

        if let Some((first, rest)) = parts.split_first()
            && let Some(crate_data) = self.dependencies.get_mut(*first)
        {
            return descend(crate_data, rest, &info);
        }

        descend(&mut self.current, &parts, &info)
    }

    pub(crate) fn import_use_items(
        &mut self,
        module_path: &[String],
        p: &TranslationUnit<StatelessResolver>,
    ) {
        let mut errors: Vec<co2_ast::Rich<'static, String, co2_ast::Span>> = Vec::new();
        let info = self.dep_info();
        for (use_item, _) in &p.rust_use_items {
            let Some((last_segment, _)) = use_item.path.last() else {
                continue;
            };

            if last_segment == "*" {
                let Some(item) = self.resolve_module_node_relative(
                    module_path,
                    use_item.path[..use_item.path.len() - 1]
                        .iter()
                        .map(|(segment, _)| segment.as_str()),
                ) else {
                    if let Some((_, span)) =
                        self.first_unresolved_use_segment(&info, module_path, use_item)
                    {
                        errors.push(co2_ast::Rich::custom(*span, "Unresolved item".to_owned()));
                    }
                    continue;
                };

                if let ModuleData::Expanded(ref content) = item {
                    for (name, child_item) in &content.items {
                        let target = self.module_mut(module_path);
                        match target {
                            ModuleData::Expanded(c) => {
                                c.items.entry(name.clone()).or_insert(child_item.clone());
                            }
                            ModuleData::Unexpanded(_) => unreachable!(),
                        }
                    }
                }
                continue;
            }

            let alias = if let Some((alias_name, _)) = &use_item.alias {
                alias_name.as_str()
            } else {
                last_segment.as_str()
            };
            let module = self.module_mut(module_path);
            if module.resolve_path([alias].into_iter(), &info).is_some() {
                continue;
            }

            let full_path = use_item
                .path
                .iter()
                .map(|(segment, _)| segment.as_str())
                .collect::<Vec<_>>()
                .join("::");
            let normalized_full_path = normalized_path(&full_path);

            let Some(item) = self.resolve_module_node_relative(
                module_path,
                use_item.path.iter().map(|(segment, _)| segment.as_str()),
            ) else {
                if let Some((_, span)) =
                    self.first_unresolved_use_segment(&info, module_path, use_item)
                {
                    errors.push(co2_ast::Rich::custom(*span, "Unresolved item".to_owned()));
                }
                continue;
            };
            let item_id = match &item {
                ModuleData::Expanded(c) => c.id,
                ModuleData::Unexpanded(_) => None,
            };
            if let Some((def_id, TypeQueryResult::Type)) = item_id
                && info.is_trait(def_id)
            {
                self.scoped_traits.push(ScopedTrait {
                    name: alias.to_owned(),
                    def_id,
                    path: normalized_full_path.clone(),
                });
            }
            let item = if matches!(item_id, Some((_, TypeQueryResult::Expr))) {
                ModuleData::Expanded(Box::new(ModuleContent {
                    id: item_id,
                    ..Default::default()
                }))
            } else {
                item
            };
            self.module_mut(module_path).insert_alias(alias, item);
        }
        if !errors.is_empty() {
            co2_ast::emit_errors(errors);
        }
    }

    fn first_unresolved_use_segment<'a>(
        &mut self,
        _info: &DependencyInfo<'_>,
        module_path: &[String],
        use_item: &'a co2_ast::UseItem,
    ) -> Option<&'a co2_ast::Spanned<String>> {
        let parts = use_item
            .path
            .iter()
            .map(|(segment, _)| segment.as_str())
            .collect::<Vec<_>>();
        if let Some((prefix, rest)) = anchored_module_prefix(module_path, &parts) {
            let mut current = &self.current;
            for segment in &prefix {
                match current {
                    ModuleData::Expanded(c) => {
                        let Some(next) = c.items.get(*segment) else {
                            return use_item
                                .path
                                .iter()
                                .find(|(name, _)| name.as_str() == *segment);
                        };
                        current = next;
                    }
                    ModuleData::Unexpanded(_) => return None,
                }
            }
            for segment in rest {
                match current {
                    ModuleData::Expanded(c) => {
                        let Some(next) = c.items.get(*segment) else {
                            return use_item
                                .path
                                .iter()
                                .find(|(name, _)| name.as_str() == *segment);
                        };
                        current = next;
                    }
                    ModuleData::Unexpanded(_) => {
                        return use_item
                            .path
                            .iter()
                            .find(|(name, _)| name.as_str() == *segment);
                    }
                }
            }
            return None;
        }
        let (first, rest) = use_item.path.split_first()?;
        if let Some(module) = self.dependencies.get(first.0.as_str()) {
            let mut current = module;
            for segment in rest {
                match current {
                    ModuleData::Expanded(c) => {
                        let Some(next) = c.items.get(&segment.0) else {
                            return Some(segment);
                        };
                        current = next;
                    }
                    ModuleData::Unexpanded(_) => return Some(segment),
                }
            }
            return None;
        }
        let mut current = &self.current;
        for segment in &use_item.path {
            match current {
                ModuleData::Expanded(c) => {
                    let Some(next) = c.items.get(&segment.0) else {
                        return Some(segment);
                    };
                    current = next;
                }
                ModuleData::Unexpanded(_) => return Some(segment),
            }
        }
        None
    }

    pub(crate) fn insert_module_data(&mut self, path: &[String], alias: &str, item: ModuleData) {
        fn seed_builtin_aliases(root: &ModuleContent, module: &mut ModuleContent) {
            if module.id.is_some() {
                return;
            }
            for name in ["__builtin_va_list", "__gnuc_va_list"] {
                if let Some(item) = root.items.get(name).cloned() {
                    module.items.entry(name.to_owned()).or_insert(item);
                }
            }
            for child in module.items.values_mut() {
                if let ModuleData::Expanded(c) = child {
                    seed_builtin_aliases(root, c);
                }
            }
        }

        let root_content = match &self.current {
            ModuleData::Expanded(c) => c.as_ref(),
            ModuleData::Unexpanded(_) => panic!("current must be expanded"),
        };

        let mut item = item;
        if let ModuleData::Expanded(ref mut item_content) = item {
            seed_builtin_aliases(root_content, item_content);
        }
        let mut module = &mut self.current;
        for segment in path {
            module = module
                .as_content_mut()
                .items
                .entry(segment.clone())
                .or_default();
        }
        module.insert_alias(alias, item);
    }

    pub(crate) fn resolve_in_deps<'a>(
        &mut self,
        crate_name: &str,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        let info = self.dep_info();
        let parts: Vec<&str> = path.into_iter().collect();
        let crate_data = self.dependencies.get_mut(crate_name)?;
        crate_data.resolve_path_inner(&parts, &info)
    }

    pub(crate) fn resolve_in_current<'a>(
        &mut self,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        let info = self.dep_info();
        let parts: Vec<&str> = path.into_iter().collect();
        self.current.resolve_path_inner(&parts, &info)
    }

    pub fn resolve(
        &mut self,
        path: &str,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), ResolveError> {
        let parts: Vec<&str> = path.split("::").collect();
        let result = if let Some((crate_name, _)) = path.split_once("::") {
            if self.dependencies.contains_key(crate_name) {
                self.resolve_in_deps(crate_name, parts[1..].iter().copied())
            } else {
                self.resolve_in_current(parts.iter().copied())
            }
        } else {
            self.resolve_in_current(parts.iter().copied())
        };
        match result {
            Some(found) => Ok(found),
            None => Err(ResolveError),
        }
    }

    pub(crate) fn resolve_relative_expr_path(
        &mut self,
        module_path: &[String],
        path: &str,
    ) -> Option<ResolvedExprPath> {
        let (def_id, class) = self.resolve_relative(module_path, path)?;
        Some(ResolvedExprPath::Def(def_id, class))
    }

    pub(crate) fn resolve_relative(
        &mut self,
        module_path: &[String],
        path: &str,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        let parts: Vec<&str> = path.split("::").collect();
        let first = parts.first().copied()?;

        if let Some((prefix, rest)) = anchored_module_prefix(module_path, &parts) {
            let info = self.dep_info();
            let mut current = &mut self.current;
            for segment in &prefix {
                current = current.as_content_mut().items.get_mut(*segment)?;
            }
            return current.resolve_path(rest.iter().copied(), &info);
        }

        if parts.len() > 1 && self.dependencies.contains_key(first) {
            return self.resolve(path).ok();
        }

        for prefix_len in (0..=module_path.len()).rev() {
            let prefix = module_path[..prefix_len].iter().map(String::as_str);
            let info = self.dep_info();
            let mut current = &mut self.current;
            for segment in prefix {
                current = current.as_content_mut().items.get_mut(segment)?;
            }
            if let Some(found) = current.resolve_path(parts.iter().copied(), &info) {
                return Some(found);
            }
        }

        self.resolve(path).ok()
    }

    /// Lower inherent methods where rustc infra is not available.
    pub(crate) fn resolve_inherent_method_for_sig(
        &mut self,
        receiver_ty: Ty,
        method: &str,
    ) -> Option<(DefId, TypeQueryResult)> {
        let ty_def_id = match receiver_ty.kind() {
            TyKind::RigidTy(RigidTy::Adt(adt, _)) => Some(adt.0),
            TyKind::RigidTy(RigidTy::Ref(_, inner, _)) => {
                return self.resolve_inherent_method_for_sig(inner, method);
            }
            TyKind::RigidTy(RigidTy::RawPtr(inner, _)) => {
                if let Some(found) = self.resolve_inherent_method_for_sig(inner, method) {
                    return Some(found);
                }
                None
            }
            _ => None,
        };

        if let Some(ty_def_id) = ty_def_id
            && let Some(module) = self.method_receivers.get(&ty_def_id)
            && let Some(found) = Self::resolve_method_in_module(module, method)
        {
            return Some(found);
        }

        let info = self.dep_info();
        if let Some(ty_def_id) = ty_def_id {
            for impl_fn in info.impls(ty_def_id) {
                let name = impl_fn.name.split("::").last().unwrap_or(&impl_fn.name);
                if name == method {
                    return Some((impl_fn.def_id, TypeQueryResult::Expr));
                }
            }
        }
        for impl_fn in info.incoherent_impls(receiver_ty) {
            let name = impl_fn.name.split("::").last().unwrap_or(&impl_fn.name);
            if name == method {
                return Some((impl_fn.def_id, TypeQueryResult::Expr));
            }
        }
        None
    }

    pub(crate) fn traits_in_scope_with_method(
        &mut self,
        method: &str,
    ) -> Vec<(String, DefId, String)> {
        let mut out = Vec::new();
        let mut seen_paths: HashSet<&str> = HashSet::new();
        let info = self.dep_info();
        for scoped_trait in &self.scoped_traits {
            if !seen_paths.insert(scoped_trait.path.as_str()) {
                continue;
            }
            let has_method = info
                .children(scoped_trait.def_id)
                .iter()
                .any(|child| child.kind == DependencyChildKind::Function && child.name == method);
            if has_method {
                out.push((
                    scoped_trait.name.clone(),
                    scoped_trait.def_id,
                    scoped_trait.path.clone(),
                ));
            }
        }
        out
    }

    pub(crate) fn resolve_trait_method(
        &mut self,
        trait_path: &str,
        method: &str,
    ) -> Option<(DefId, TypeQueryResult)> {
        let (trait_def_id, _) = self.resolve(trait_path).ok()?;
        let info = self.dep_info();
        for child in info.children(trait_def_id) {
            if child.kind == DependencyChildKind::Function && child.name == method {
                return Some((child.def_id, TypeQueryResult::Expr));
            }
        }
        None
    }

    fn resolve_method_in_module(
        module: &ModuleData,
        method: &str,
    ) -> Option<(DefId, TypeQueryResult)> {
        let content = match module {
            ModuleData::Expanded(c) => c,
            ModuleData::Unexpanded(_) => return None,
        };
        if let Some((_, _)) = content.id
            && content.items.contains_key(method)
            && let ModuleData::Expanded(child) = &content.items[method]
        {
            return child.id;
        }
        let mut children: Vec<_> = content.items.iter().collect();
        children.sort_by_key(|(name, _)| method_search_priority(name));
        for (_, child) in children {
            if let Some(found) = Self::resolve_method_in_module(child, method) {
                return Some(found);
            }
        }
        None
    }

    pub(crate) fn rebuild_method_receivers(&mut self) {
        self.method_receivers.clear();
        Self::collect_method_receivers(&self.current, &mut self.method_receivers);
    }

    fn collect_method_receivers(module: &ModuleData, out: &mut HashMap<DefId, ModuleData>) {
        let content = match module {
            ModuleData::Expanded(c) => c,
            ModuleData::Unexpanded(_) => return,
        };
        if let Some((def_id, TypeQueryResult::Type)) = content.id
            && !(content.items.is_empty() && out.contains_key(&def_id))
        {
            out.insert(def_id, module.clone());
        }
        for child in content.items.values() {
            Self::collect_method_receivers(child, out);
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResolvedExprPath {
    Def(DefId, TypeQueryResult),
}

fn normalized_path(path: &str) -> String {
    let Some((mut crate_name, rest)) = path.split_once("::") else {
        return path.to_owned();
    };
    normalize_crate_name(&mut crate_name);
    format!("{crate_name}::{rest}")
}

fn method_search_priority(name: &str) -> (usize, &str) {
    let generic_arity = name
        .strip_prefix('<')
        .and_then(|it| it.strip_suffix('>'))
        .map_or(usize::MAX, |inner| {
            inner
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .count()
        });
    (generic_arity, name)
}
