use std::collections::{HashMap, HashSet};

use co2_ast::{Declaration, Declarator, StatelessResolver, TranslationUnit, TypeQueryResult};
use rustc_public_generative::{
    DefData, DependencyInfo, DependencyValueKind, HirStructureCtx,
    rustc_public::{
        DefId,
        ty::{RigidTy, Ty, TyKind},
    },
};

#[derive(Debug, Default, Clone)]
pub(crate) struct ModuleData {
    id: Option<(DefId, TypeQueryResult)>,
    items: HashMap<String, ModuleData>,
}

#[derive(Debug, Clone)]
struct ScopedTrait {
    name: String,
    def_id: DefId,
    path: String,
}

#[derive(Debug, Clone)]
struct TraitMethod {
    trait_path: String,
    method: String,
    def_id: DefId,
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

impl ModuleData {
    fn insert_path<'a>(
        &mut self,
        mut path: impl Iterator<Item = &'a str>,
        def: Option<(DefId, TypeQueryResult)>,
    ) {
        let Some(seg1) = path.next() else {
            self.id = def;
            return;
        };
        let part = self.items.entry(seg1.to_owned()).or_default();
        part.insert_path(path, def);
    }

    fn resolve_path<'a>(
        &self,
        path: impl Iterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        let parts = path.collect::<Vec<_>>();
        self.lookup_path(&parts)?
            .id
            .ok_or_else(|| "path does not refer to a value or type".to_owned())
    }

    fn resolve_path_or_unique_leaf<'a>(
        &self,
        path: impl Iterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        let parts = path.collect::<Vec<_>>();
        self.lookup_path_or_unique_leaf(&parts)?
            .id
            .ok_or_else(|| "path does not refer to a value or type".to_owned())
    }

    fn lookup_path<'a>(&self, path: &[&'a str]) -> Result<&Self, String> {
        let Some((seg1, rest)) = path.split_first() else {
            return Ok(self);
        };
        let part = self.items.get(*seg1).ok_or_else(|| {
            format!(
                "Failed to lookup {seg1}.\nAvailable items are: {:?}",
                self.items.keys()
            )
        })?;
        part.lookup_path(rest)
    }

    fn lookup_path_or_unique_leaf<'a>(&'a self, path: &[&'a str]) -> Result<&'a Self, String> {
        match self.lookup_path(path) {
            Ok(found) => Ok(found),
            Err(err) if path.len() == 1 => self.lookup_unique_leaf(path[0]).or(Err(err)),
            Err(err) => Err(err),
        }
    }

    // TODO: this function is super wrong. It's just a workaround for
    // having `libc::puts` (not `libc::unix::puts`) without supporting
    // `pub use` in dependencies properly.
    fn lookup_unique_leaf<'a>(&'a self, leaf: &str) -> Result<&'a Self, String> {
        let mut matches = Vec::new();
        self.collect_leaf_matches(leaf, &mut matches);
        match matches.as_slice() {
            [] => Err(format!(
                "Failed to lookup {leaf}.\nAvailable items are: {:?}",
                self.items.keys()
            )),
            [single] => Ok(*single),
            many => {
                let first_id = many[0].id;
                if first_id.is_some() && many.iter().all(|item| item.id == first_id) {
                    Ok(many[0])
                } else {
                    Err(format!(
                        "lookup for `{leaf}` is ambiguous in dependency tree"
                    ))
                }
            }
        }
    }

    fn collect_leaf_matches<'a>(&'a self, leaf: &str, out: &mut Vec<&'a Self>) {
        if let Some(child) = self.items.get(leaf) {
            out.push(child);
        }
        for child in self.items.values() {
            child.collect_leaf_matches(leaf, out);
        }
    }

    pub(crate) fn insert_alias(&mut self, alias: &str, item: ModuleData) {
        self.items.insert(alias.to_owned(), item);
    }

    pub(crate) fn forward_pass_parsed_module(
        ctx: &HirStructureCtx<'_>,
        ast: &TranslationUnit<StatelessResolver>,
        parent: DefId,
        foreign_mod: DefId,
        include_builtin_va_list: bool,
    ) -> Self {
        let mut this = Self::default();
        if include_builtin_va_list {
            for name in ["__builtin_va_list", "__gnuc_va_list"] {
                let def_id = ctx.allocate_def_id(parent, DefData::TypeNs(name.to_owned()));
                this.insert_path([name].into_iter(), Some((def_id, TypeQueryResult::Type)));
            }
        }
        for (item, _) in &ast.items {
            match item {
                Declaration::FunctionDefinition { signature, .. } => {
                    let Some(decl) = signature.ident() else {
                        continue;
                    };
                    let def_id = ctx.allocate_def_id(parent, DefData::ValueNs(decl.clone()));
                    this.insert_path([&*decl].into_iter(), Some((def_id, TypeQueryResult::Expr)));
                }
                Declaration::Declaration {
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
                            let Some(name) = extract_decl_name(&decl) else {
                                continue;
                            };
                            if this.resolve_path([&*name].into_iter()).is_ok() {
                                continue;
                            }
                            let def_id = ctx.allocate_def_id(parent, DefData::TypeNs(name.clone()));
                            this.insert_path(
                                [&*name].into_iter(),
                                Some((def_id, TypeQueryResult::Type)),
                            );
                        }
                    } else {
                        for decl in declarators {
                            let decl = &decl.0.declarator.0;
                            let Some(name) = extract_decl_name(&decl) else {
                                continue;
                            };
                            if this.resolve_path([&*name].into_iter()).is_ok() {
                                continue;
                            }
                            let parent = if decl.is_function() || is_extern {
                                foreign_mod
                            } else {
                                parent
                            };
                            let def_id =
                                ctx.allocate_def_id(parent, DefData::ValueNs(name.clone()));
                            this.insert_path(
                                [&*name].into_iter(),
                                Some((def_id, TypeQueryResult::Expr)),
                            );
                        }
                    }
                }
            }
        }
        this
    }
}

fn anchored_module_prefix<'a>(
    module_path: &'a [String],
    parts: &'a [&'a str],
) -> Option<(Vec<&'a str>, &'a [&'a str])> {
    let Some(first) = parts.first().copied() else {
        return None;
    };
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

#[derive(Debug, Default)]
pub struct Resolver {
    const_values: HashMap<String, DefId>,
    dependencies: HashMap<String, ModuleData>,
    current: ModuleData,
    method_receivers: HashMap<DefId, ModuleData>,
    traits: HashSet<DefId>,
    scoped_traits: Vec<ScopedTrait>,
    trait_methods: HashMap<String, Vec<TraitMethod>>,
}

fn normalize_crate_name(name: &mut &str) {
    if *name == "std" || *name == "core" || *name == "alloc" {
        *name = "std_core";
    }
}

impl Resolver {
    pub(crate) fn new(
        ctx: &HirStructureCtx<'_>,
        deps: DependencyInfo,
        p: &TranslationUnit<StatelessResolver>,
        foreign_mod: DefId,
    ) -> Self {
        let mut this = Self::default();
        for c in deps.crates {
            this.dependencies
                .insert(c.name.clone(), ModuleData::default());
        }
        this.dependencies
            .insert("std_core".to_owned(), ModuleData::default());
        for t in &deps.traits {
            this.traits.insert(t.def_id);
            let (mut crate_name, rest) = t.path.split_once("::").unwrap();
            normalize_crate_name(&mut crate_name);
            let Some(i) = this.dependencies.get_mut(crate_name) else {
                continue;
            };
            i.insert_path(rest.split("::"), Some((t.def_id, TypeQueryResult::Type)));
        }
        for t in deps.functions {
            if let Some(fn_def) = t.fn_def
                && let Some((trait_path, method)) = parse_trait_method_path(&t.path)
                && self::Resolver::is_known_trait_path(&this, &trait_path)
            {
                this.trait_methods
                    .entry(method.clone())
                    .or_default()
                    .push(TraitMethod {
                        trait_path,
                        method,
                        def_id: fn_def.0,
                    });
            }
            let (mut crate_name, rest) = t.path.split_once("::").unwrap();
            normalize_crate_name(&mut crate_name);
            let Some(i) = this.dependencies.get_mut(crate_name) else {
                continue;
            };
            i.insert_path(
                rest.split("::"),
                t.fn_def.map(|x| (x.0, TypeQueryResult::Expr)),
            );
        }
        for t in deps.values {
            match t.kind {
                DependencyValueKind::Def(def_id) => {
                    let (mut crate_name, rest) = t.path.split_once("::").unwrap();
                    normalize_crate_name(&mut crate_name);
                    let Some(i) = this.dependencies.get_mut(crate_name) else {
                        continue;
                    };
                    i.insert_path(rest.split("::"), Some((def_id, TypeQueryResult::Expr)));
                }
                DependencyValueKind::ConstDef(def_id) => {
                    this.const_values.insert(normalized_path(&t.path), def_id);
                }
            }
        }
        for t in deps.types {
            let (mut crate_name, rest) = t.path.split_once("::").unwrap();
            normalize_crate_name(&mut crate_name);
            let Some(i) = this.dependencies.get_mut(crate_name) else {
                continue;
            };
            i.insert_path(rest.split("::"), Some((t.adt.0, TypeQueryResult::Type)));
        }
        this.current = ModuleData::forward_pass_parsed_module(
            ctx,
            p,
            ctx.root_crate_def_id(),
            foreign_mod,
            true,
        );
        this.current.insert_path(
            ["__builtin_bswap16"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u16>", "swap_bytes"])
                    .unwrap(),
            ),
        );
        this.current.insert_path(
            ["__builtin_bswap32"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u32>", "swap_bytes"])
                    .unwrap(),
            ),
        );
        this.current.insert_path(
            ["__builtin_bswap64"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u64>", "swap_bytes"])
                    .unwrap(),
            ),
        );
        // __builtin_clz: count leading zeros for unsigned int (u32)
        this.current.insert_path(
            ["__builtin_clz"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u32>", "leading_zeros"])
                    .unwrap(),
            ),
        );
        // __builtin_clzll: count leading zeros for unsigned long long (u64)
        this.current.insert_path(
            ["__builtin_clzll"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u64>", "leading_zeros"])
                    .unwrap(),
            ),
        );
        // __builtin_ctz: count trailing zeros for unsigned int (u32)
        this.current.insert_path(
            ["__builtin_ctz"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u32>", "trailing_zeros"])
                    .unwrap(),
            ),
        );
        // __builtin_ctzll: count trailing zeros for unsigned long long (u64)
        this.current.insert_path(
            ["__builtin_ctzll"].into_iter(),
            Some(
                this.resolve_in_deps("std", ["num", "<impl u64>", "trailing_zeros"])
                    .unwrap(),
            ),
        );
        this.rebuild_method_receivers();
        this
    }

    fn module_mut<'a>(&'a mut self, path: &[String]) -> &'a mut ModuleData {
        let mut module = &mut self.current;
        for segment in path {
            module = module.items.entry(segment.clone()).or_default();
        }
        module
    }

    fn resolve_module_path_relative<'a>(
        &self,
        module_path: &[String],
        path: impl IntoIterator<Item = &'a str>,
    ) -> Result<ModuleData, String> {
        let parts = path.into_iter().collect::<Vec<_>>();
        let Some(first) = parts.first().copied() else {
            return Err("empty path".to_owned());
        };
        if let Some((prefix, rest)) = anchored_module_prefix(module_path, &parts) {
            return self
                .current
                .lookup_path(
                    &prefix
                        .into_iter()
                        .chain(rest.iter().copied())
                        .collect::<Vec<_>>(),
                )
                .cloned();
        }
        let mut crate_name = first;
        normalize_crate_name(&mut crate_name);
        if let Some(crate_data) = self.dependencies.get(crate_name) {
            return crate_data.lookup_path_or_unique_leaf(&parts[1..]).cloned();
        }
        self.current.lookup_path(&parts).cloned()
    }

    pub(crate) fn import_use_items(
        &mut self,
        module_path: &[String],
        p: &TranslationUnit<StatelessResolver>,
    ) {
        let mut errors: Vec<co2_ast::Rich<'static, String, co2_ast::Span>> = Vec::new();
        for (use_item, _) in &p.rust_use_items {
            let Some((last_segment, _)) = use_item.path.last() else {
                continue;
            };

            if last_segment == "*" {
                let Ok(item) = self.resolve_module_path_relative(
                    module_path,
                    use_item.path[..use_item.path.len() - 1]
                        .iter()
                        .map(|(segment, _)| segment.as_str()),
                ) else {
                    if let Some((_, span)) =
                        self.first_unresolved_use_segment(module_path, use_item)
                    {
                        errors.push(co2_ast::Rich::custom(*span, "Unresolved item".to_owned()));
                    }
                    continue;
                };

                for (name, child_item) in item.items {
                    self.module_mut(module_path).insert_alias(&name, child_item);
                }
                continue;
            }

            let alias = if let Some((alias_name, _)) = &use_item.alias {
                alias_name.as_str()
            } else {
                last_segment.as_str()
            };
            let module = self.module_mut(module_path);
            if module.resolve_path([alias].into_iter()).is_ok()
                || self.const_values.contains_key(alias)
            {
                continue;
            }
            let full_path = use_item
                .path
                .iter()
                .map(|(segment, _)| segment.as_str())
                .collect::<Vec<_>>()
                .join("::");
            let normalized_full_path = normalized_path(&full_path);
            if let Some(def_id) = self.const_values.get(&normalized_path(&full_path)).copied() {
                self.const_values.insert(alias.to_owned(), def_id);
                continue;
            }
            let Ok(item) = self.resolve_module_path_relative(
                module_path,
                use_item.path.iter().map(|(segment, _)| segment.as_str()),
            ) else {
                if let Some((_, span)) = self.first_unresolved_use_segment(module_path, use_item) {
                    errors.push(co2_ast::Rich::custom(*span, "Unresolved item".to_owned()));
                }
                continue;
            };
            if let Some((def_id, TypeQueryResult::Type)) = item.id {
                if self.traits.contains(&def_id) {
                    self.scoped_traits.push(ScopedTrait {
                        name: alias.to_owned(),
                        def_id,
                        path: normalized_full_path.clone(),
                    });
                }
            }
            let item = if matches!(item.id, Some((_, TypeQueryResult::Expr))) {
                ModuleData {
                    id: item.id,
                    items: HashMap::new(),
                }
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
        &self,
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
            for segment in prefix {
                let Some(next) = current.items.get(segment) else {
                    return use_item
                        .path
                        .iter()
                        .find(|(name, _)| name.as_str() == segment);
                };
                current = next;
            }
            for segment in rest {
                let Some(next) = current.items.get(*segment) else {
                    return use_item
                        .path
                        .iter()
                        .find(|(name, _)| name.as_str() == *segment);
                };
                current = next;
            }
            return None;
        }
        let (first, rest) = use_item.path.split_first()?;
        let mut crate_name = first.0.as_str();
        normalize_crate_name(&mut crate_name);
        if let Some(module) = self.dependencies.get(crate_name) {
            let mut current = module;
            for segment in rest {
                let Some(next) = current.items.get(&segment.0) else {
                    return Some(segment);
                };
                current = next;
            }
            return None;
        }
        let mut current = &self.current;
        for segment in &use_item.path {
            let Some(next) = current.items.get(&segment.0) else {
                return Some(segment);
            };
            current = next;
        }
        None
    }

    pub(crate) fn insert_module_data(&mut self, path: &[String], alias: &str, item: ModuleData) {
        fn seed_builtin_aliases(root: &ModuleData, module: &mut ModuleData) {
            if module.id.is_some() {
                return;
            }
            for name in ["__builtin_va_list", "__gnuc_va_list"] {
                if let Some(item) = root.items.get(name).cloned() {
                    module.items.entry(name.to_owned()).or_insert(item);
                }
            }
            for child in module.items.values_mut() {
                seed_builtin_aliases(root, child);
            }
        }

        let mut item = item;
        seed_builtin_aliases(&self.current, &mut item);
        let mut module = &mut self.current;
        for segment in path {
            module = module.items.entry(segment.clone()).or_default();
        }
        module.insert_alias(alias, item);
    }

    pub(crate) fn resolve_in_deps<'a>(
        &self,
        mut crate_name: &str,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        normalize_crate_name(&mut crate_name);
        let crate_data = self
            .dependencies
            .get(crate_name)
            .ok_or_else(|| format!("Crate {crate_name} not found"))?;
        crate_data.resolve_path_or_unique_leaf(path.into_iter())
    }

    pub(crate) fn resolve_in_current<'a>(
        &self,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        self.current.resolve_path(path.into_iter())
    }

    pub fn resolve(&self, path: &str) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        let Some((mut crate_name, rest)) = path.split_once("::") else {
            return self.resolve_in_current([path]);
        };
        normalize_crate_name(&mut crate_name);
        if self.dependencies.contains_key(crate_name) {
            self.resolve_in_deps(crate_name, rest.split("::"))
        } else {
            self.resolve_in_current(path.split("::"))
        }
    }

    pub(crate) fn resolve_relative_expr_path(
        &self,
        module_path: &[String],
        path: &str,
    ) -> Result<ResolvedExprPath, String> {
        if let Some(def_id) = self.const_values.get(&normalized_path(path)) {
            return Ok(ResolvedExprPath::Const(*def_id));
        }
        let (def_id, class) = self.resolve_relative(module_path, path)?;
        Ok(ResolvedExprPath::Def(def_id, class))
    }

    pub(crate) fn resolve_relative(
        &self,
        module_path: &[String],
        path: &str,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        let parts = path.split("::").collect::<Vec<_>>();
        let Some(first) = parts.first().copied() else {
            return Err("empty path".to_owned());
        };

        if let Some((prefix, rest)) = anchored_module_prefix(module_path, &parts) {
            return self
                .current
                .resolve_path(prefix.into_iter().chain(rest.iter().copied()));
        }

        // Only treat the first segment as a crate name when the path has `::` separators.
        // A bare identifier (e.g. `memchr`) can never be a crate path — crate references
        // always look like `crate_name::item`. Without this guard, a C extern whose name
        // collides with a Rust dependency crate (e.g. the `memchr` crate) would be resolved
        // against the crate instead of the current module scope.
        let mut crate_name = first;
        normalize_crate_name(&mut crate_name);
        if parts.len() > 1 && self.dependencies.contains_key(crate_name) {
            return self.resolve(path);
        }

        for prefix_len in (0..=module_path.len()).rev() {
            let prefix = module_path[..prefix_len].iter().map(String::as_str);
            if let Ok(found) = self
                .current
                .resolve_path(prefix.chain(parts.iter().copied()))
            {
                return Ok(found);
            }
        }

        self.resolve(path)
    }

    pub(crate) fn resolve_inherent_method(
        &self,
        receiver_ty: Ty,
        method: &str,
    ) -> Result<Option<(DefId, TypeQueryResult)>, String> {
        match receiver_ty.kind() {
            TyKind::RigidTy(RigidTy::Adt(adt, _)) => Ok(self
                .method_receivers
                .get(&adt.0)
                .and_then(|module| Self::resolve_method_in_module(module, method).ok())),
            TyKind::RigidTy(RigidTy::Ref(_, inner, _) | RigidTy::RawPtr(inner, _)) => {
                self.resolve_inherent_method(inner, method)
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn traits_in_scope_with_method(&self, method: &str) -> Vec<(String, DefId, String)> {
        let mut out = Vec::new();
        for scoped_trait in &self.scoped_traits {
            if !self.trait_methods.get(method).is_some_and(|candidates| {
                candidates
                    .iter()
                    .any(|candidate| candidate.trait_path == scoped_trait.path)
            }) {
                continue;
            }
            out.push((
                scoped_trait.name.clone(),
                scoped_trait.def_id,
                scoped_trait.path.clone(),
            ));
        }
        out
    }

    pub(crate) fn resolve_trait_method(
        &self,
        trait_path: &str,
        method: &str,
    ) -> Option<(DefId, TypeQueryResult)> {
        self.trait_methods
            .get(method)?
            .iter()
            .find_map(|candidate| {
                (candidate.trait_path == trait_path && candidate.method == method)
                    .then_some((candidate.def_id, TypeQueryResult::Expr))
            })
    }

    fn is_known_trait_path(&self, path: &str) -> bool {
        self.resolve(path)
            .is_ok_and(|(_, class)| class == TypeQueryResult::Type)
    }

    fn resolve_method_in_module(
        module: &ModuleData,
        method: &str,
    ) -> Result<(DefId, TypeQueryResult), String> {
        if let Ok(found) = module.resolve_path([method].into_iter()) {
            return Ok(found);
        }
        let mut children = module.items.iter().collect::<Vec<_>>();
        children.sort_by_key(|(name, _)| method_search_priority(name));
        for (_, child) in children {
            if let Ok(found) = Self::resolve_method_in_module(child, method) {
                return Ok(found);
            }
        }
        Err(format!(
            "Failed to lookup {method}.\nAvailable items are: {:?}",
            module.items.keys()
        ))
    }

    pub(crate) fn rebuild_method_receivers(&mut self) {
        self.method_receivers.clear();
        for module in self.dependencies.values() {
            Self::collect_method_receivers(module, &mut self.method_receivers);
        }
        Self::collect_method_receivers(&self.current, &mut self.method_receivers);
    }

    fn collect_method_receivers(module: &ModuleData, out: &mut HashMap<DefId, ModuleData>) {
        if let Some((def_id, TypeQueryResult::Type)) = module.id {
            if !(module.items.is_empty() && out.contains_key(&def_id)) {
                out.insert(def_id, module.clone());
            }
        }
        for child in module.items.values() {
            Self::collect_method_receivers(child, out);
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResolvedExprPath {
    Def(DefId, TypeQueryResult),
    Const(DefId),
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
        .map(|inner| {
            inner
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .count()
        })
        .unwrap_or(usize::MAX);
    (generic_arity, name)
}

fn parse_trait_method_path(path: &str) -> Option<(String, String)> {
    if path.starts_with('<') {
        return None;
    }
    let (trait_path, method) = path.rsplit_once("::")?;
    Some((normalized_path(trait_path), method.to_owned()))
}
