use std::collections::HashMap;

use co2_ast::{Declaration, Declarator, StatelessResolver, TranslationUnit, TypeQueryResult};
use rustc_public_generative::{
    DefData, DependencyInfo, DependencyValueKind, HirStructureCtx,
    rustc_public::{
        DefId,
        ty::{RigidTy, Ty, TyKind},
    },
};

#[derive(Debug, Default, Clone)]
struct ModuleData {
    id: Option<(DefId, TypeQueryResult)>,
    items: HashMap<String, ModuleData>,
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
        self.lookup_path(path)?
            .id
            .ok_or_else(|| "path does not refer to a value or type".to_owned())
    }

    fn lookup_path<'a>(&self, mut path: impl Iterator<Item = &'a str>) -> Result<&Self, String> {
        let Some(seg1) = path.next() else {
            return Ok(self);
        };
        let part = self.items.get(seg1).ok_or_else(|| {
            format!(
                "Failed to lookup {seg1}.\nAvailable items are: {:?}",
                self.items.keys()
            )
        })?;
        part.lookup_path(path)
    }

    fn insert_alias(&mut self, alias: &str, item: ModuleData) {
        self.items.insert(alias.to_owned(), item);
    }

    fn forward_pass_parsed_module(
        ctx: &HirStructureCtx<'_>,
        ast: &TranslationUnit<StatelessResolver>,
        parent: DefId,
        foreign_mod: DefId,
    ) -> Self {
        let mut this = Self::default();
        {
            let name = "__builtin_va_list";
            let def_id = ctx.allocate_def_id(parent, DefData::TypeNs(name.to_owned()));
            this.insert_path([name].into_iter(), Some((def_id, TypeQueryResult::Type)));
        }
        for (item, _) in &ast.items {
            match item {
                Declaration::FunctionDefinition {
                    declaration_specifiers: _,
                    declarator,
                    body: _,
                } => {
                    let Some(decl) = extract_decl_name(&declarator.0) else {
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

#[derive(Debug, Default)]
pub struct Resolver {
    const_values: HashMap<String, DefId>,
    dependencies: HashMap<String, ModuleData>,
    current: ModuleData,
    method_receivers: HashMap<DefId, ModuleData>,
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
        for t in deps.functions {
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
        for t in deps.traits {
            let (mut crate_name, rest) = t.path.split_once("::").unwrap();
            normalize_crate_name(&mut crate_name);
            let Some(i) = this.dependencies.get_mut(crate_name) else {
                continue;
            };
            i.insert_path(rest.split("::"), Some((t.def_id, TypeQueryResult::Type)));
        }
        for t in deps.types {
            let (mut crate_name, rest) = t.path.split_once("::").unwrap();
            normalize_crate_name(&mut crate_name);
            let Some(i) = this.dependencies.get_mut(crate_name) else {
                continue;
            };
            i.insert_path(rest.split("::"), Some((t.adt.0, TypeQueryResult::Type)));
        }
        this.current =
            ModuleData::forward_pass_parsed_module(ctx, p, ctx.root_crate_def_id(), foreign_mod);
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
        this.import_use_items(p);
        this.rebuild_method_receivers();
        this
    }

    fn import_use_items(&mut self, p: &TranslationUnit<StatelessResolver>) {
        for (use_item, _) in &p.rust_use_items {
            let Some(alias) = use_item.path.last().map(|(segment, _)| segment.as_str()) else {
                continue;
            };
            if self.resolve_in_current([alias]).is_ok() || self.const_values.contains_key(alias) {
                continue;
            }
            let full_path = use_item
                .path
                .iter()
                .map(|(segment, _)| segment.as_str())
                .collect::<Vec<_>>()
                .join("::");
            if let Some(def_id) = self.const_values.get(&normalized_path(&full_path)).copied() {
                self.const_values.insert(alias.to_owned(), def_id);
                continue;
            }
            let Ok(item) = self.resolve_module_path(use_item.path.iter().map(|(segment, _)| segment.as_str())) else {
                continue;
            };
            self.current.insert_alias(alias, item);
        }
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
        crate_data.resolve_path(path.into_iter())
    }

    pub(crate) fn resolve_in_current<'a>(
        &self,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        self.current.resolve_path(path.into_iter())
    }

    fn resolve_module_path<'a>(
        &self,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Result<ModuleData, String> {
        let parts = path.into_iter().collect::<Vec<_>>();
        let Some(first) = parts.first().copied() else {
            return Err("empty path".to_owned());
        };
        let mut crate_name = first;
        normalize_crate_name(&mut crate_name);
        if let Some(crate_data) = self.dependencies.get(crate_name) {
            return crate_data.lookup_path(parts[1..].iter().copied()).cloned();
        }
        self.current.lookup_path(parts.iter().copied()).cloned()
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

    pub(crate) fn resolve_expr_path(&self, path: &str) -> Result<ResolvedExprPath, String> {
        if let Some(def_id) = self.const_values.get(&normalized_path(path)) {
            return Ok(ResolvedExprPath::Const(*def_id));
        }
        let (def_id, class) = self.resolve(path)?;
        Ok(ResolvedExprPath::Def(def_id, class))
    }

    pub(crate) fn resolve_method(
        &self,
        receiver_ty: Ty,
        method: &str,
    ) -> Result<(DefId, TypeQueryResult), String> {
        match receiver_ty.kind() {
            TyKind::RigidTy(RigidTy::Adt(adt, _)) => self
                .method_receivers
                .get(&adt.0)
                .ok_or_else(|| format!("no methods known for receiver type {:?}", receiver_ty))?
                .resolve_path([method].into_iter()),
            TyKind::RigidTy(RigidTy::Ref(_, inner, _) | RigidTy::RawPtr(inner, _)) => {
                self.resolve_method(inner, method)
            }
            _ => Err(format!(
                "method resolution is not supported for receiver type {:?}",
                receiver_ty
            )),
        }
    }

    fn rebuild_method_receivers(&mut self) {
        self.method_receivers.clear();
        for module in self.dependencies.values() {
            Self::collect_method_receivers(module, &mut self.method_receivers);
        }
        Self::collect_method_receivers(&self.current, &mut self.method_receivers);
    }

    fn collect_method_receivers(module: &ModuleData, out: &mut HashMap<DefId, ModuleData>) {
        if let Some((def_id, TypeQueryResult::Type)) = module.id {
            out.insert(def_id, module.clone());
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
