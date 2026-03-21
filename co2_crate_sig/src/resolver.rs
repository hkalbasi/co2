use std::collections::HashMap;

use co2_ast::{Declaration, Declarator, StatelessResolver, TranslationUnit, TypeQueryResult};
use rustc_public_generative::{DefData, DependencyInfo, HirStructureCtx, rustc_public::DefId};

#[derive(Debug, Default)]
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
        mut path: impl Iterator<Item = &'a str>,
    ) -> Result<(DefId, co2_ast::TypeQueryResult), String> {
        let Some(seg1) = path.next() else {
            return self.id.ok_or_else(|| format!("self id is None"));
        };
        let part = self
            .items
            .get(seg1)
            .ok_or_else(|| format!("Failed to lookup {seg1}"))?;
        part.resolve_path(path)
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
    dependencies: HashMap<String, ModuleData>,
    current: ModuleData,
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
        this
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
}
