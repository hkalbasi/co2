use co2_hir_mir::{MirModule, Type as HirType};
use rustc_public_generative as rustc_gen;

#[derive(Clone, Copy, Debug)]
pub struct CompileMode {
    pub no_main: bool,
    pub function_abi: rustc_gen::FunctionAbi,
    pub function_no_mangle: bool,
    pub function_is_unsafe: bool,
}

impl CompileMode {
    pub const RUST: Self = Self {
        no_main: false,
        function_abi: rustc_gen::FunctionAbi::Rust,
        function_no_mangle: false,
        function_is_unsafe: false,
    };

    pub const C: Self = Self {
        no_main: true,
        function_abi: rustc_gen::FunctionAbi::C,
        function_no_mangle: true,
        function_is_unsafe: false,
    };
}

pub(crate) fn func_item_id(name: &str) -> rustc_gen::ItemId {
    let mut hash = 0u64;
    for b in name.as_bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(*b as u64);
    }
    rustc_gen::ItemId::new(hash.max(1))
}

pub(crate) fn build_items(
    module: &MirModule,
    deps: rustc_gen::DependencyInfo,
    mode: CompileMode,
) -> rustc_gen::CurrentCrateInfo {
    let mut items = Vec::new();
    let mut entry = None;

    for func in &module.functions {
        let id = func_item_id(&func.name);
        if func.name == "main" {
            entry = Some(id);
        }
        items.push(rustc_gen::ItemInfo {
            id,
            name: func.name.clone(),
            parent: None,
            kind: rustc_gen::ItemKind::Function(rustc_gen::FunctionSignature {
                inputs: if mode.no_main {
                    vec![]
                } else {
                    func.sig
                        .params
                        .iter()
                        .map(|t| mir_ty_from_type(t, Some(module), &deps))
                        .collect()
                },
                output: if mode.no_main {
                    rustc_gen::MirTy::new_tuple(&[])
                } else {
                    mir_ty_from_type(&func.sig.ret, Some(module), &deps)
                },
                abi: mode.function_abi,
                is_unsafe: mode.function_is_unsafe,
            }),
            no_mangle: mode.function_no_mangle,
        });
    }

    for ext in &module.externs {
        let id = func_item_id(&ext.name);
        items.push(rustc_gen::ItemInfo {
            id,
            name: ext.name.clone(),
            parent: None,
            kind: rustc_gen::ItemKind::ForeignFunction(rustc_gen::FunctionSignature {
                inputs: ext
                    .sig
                    .params
                    .iter()
                    .map(|t| mir_ty_from_type(t, Some(module), &deps))
                    .collect(),
                output: mir_ty_from_type(&ext.sig.ret, Some(module), &deps),
                abi: rustc_gen::FunctionAbi::C,
                is_unsafe: true,
            }),
            no_mangle: false,
        });
    }

    rustc_gen::CurrentCrateInfo {
        crate_name: "co2".to_owned(),
        entry: if mode.no_main { None } else { entry },
        items,
        no_main: mode.no_main,
    }
}

pub(crate) fn mir_ty_from_type(
    ty: &HirType,
    module: Option<&MirModule>,
    deps: &rustc_gen::DependencyInfo,
) -> rustc_gen::MirTy {
    match ty {
        HirType::Void => rustc_gen::MirTy::new_tuple(&[]),
        HirType::Int => rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I32),
        HirType::Char => rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I8),
        HirType::Ptr(inner) => rustc_gen::MirTy::new_ptr(
            mir_ty_from_type(inner, module, deps),
            rustc_gen::MirMutability::Mut,
        ),
        HirType::Array(inner) => rustc_gen::MirTy::new_ptr(
            mir_ty_from_type(inner, module, deps),
            rustc_gen::MirMutability::Mut,
        ),
        HirType::RustPath(path) => mir_ty_from_rust_path(path, module, deps),
    }
}

pub(crate) fn mir_ty_from_rust_path(
    path: &co2_parser::RustPath,
    module: Option<&MirModule>,
    deps: &rustc_gen::DependencyInfo,
) -> rustc_gen::MirTy {
    let base = rust_path_base_string(path);
    if let Some(prim) = primitive_mir_ty(&base) {
        return prim;
    }

    let adt = dep_adt(deps, &base);
    let mut generic_args = rust_path_generic_args(path)
        .into_iter()
        .map(|arg| rustc_gen::GenericArgKind::Type(mir_ty_from_rust_path(&arg, module, deps)))
        .collect::<Vec<_>>();
    if (base == "std::vec::Vec" || base == "alloc::vec::Vec" || base.ends_with("::Vec"))
        && generic_args.len() == 1
    {
        let global = dep_adt_any(deps, &["alloc::alloc::Global", "std::alloc::Global"]);
        generic_args.push(rustc_gen::GenericArgKind::Type(
            rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                global,
                rustc_gen::GenericArgs(vec![]),
            )),
        ));
    }
    rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
        adt,
        rustc_gen::GenericArgs(generic_args),
    ))
}

fn rust_path_base_string(path: &co2_parser::RustPath) -> String {
    path.segments
        .iter()
        .filter_map(|seg| match &seg.0 {
            co2_parser::RustPathSegment::Ident(s) => Some(s.clone()),
            co2_parser::RustPathSegment::Generics(_) => None,
        })
        .collect::<Vec<_>>()
        .join("::")
}

pub(crate) fn rust_path_generic_args(path: &co2_parser::RustPath) -> Vec<co2_parser::RustPath> {
    for seg in &path.segments {
        if let co2_parser::RustPathSegment::Generics(args) = &seg.0 {
            return args.iter().map(|arg| arg.0.clone()).collect();
        }
    }
    Vec::new()
}

fn primitive_mir_ty(name: &str) -> Option<rustc_gen::MirTy> {
    match name {
        "u8" => Some(rustc_gen::MirTy::unsigned_ty(rustc_gen::PublicUintTy::U8)),
        "i8" => Some(rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I8)),
        "u32" => Some(rustc_gen::MirTy::unsigned_ty(rustc_gen::PublicUintTy::U32)),
        "i32" => Some(rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I32)),
        "usize" => Some(rustc_gen::MirTy::usize_ty()),
        "isize" => Some(rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::Isize)),
        _ => None,
    }
}

pub(crate) fn dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> rustc_gen::FnDef {
    if let Some(found) = find_dep_fn(deps, path) {
        if std::env::var("CO2_DEBUG_DEP").is_ok() {
            eprintln!("dep_fn resolved: {path}");
        }
        return found;
    }

    if let Some(last) = path.rsplit("::").next() {
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| f.path.ends_with(&format!("::{last}")) && f.fn_def.is_some())
            .and_then(|f| f.fn_def)
        {
            if std::env::var("CO2_DEBUG_DEP").is_ok() {
                eprintln!("dep_fn fallback resolved by suffix: {path} -> ::{last}");
            }
            return found;
        }
    }

    panic!("missing dependency function: {path}");
}

pub(crate) fn dep_fn_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> rustc_gen::FnDef {
    for path in paths {
        if let Some(found) = find_dep_fn(deps, path) {
            return found;
        }
    }
    if let Some(last) = paths.iter().find_map(|p| p.rsplit("::").next()) {
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                f.path.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && paths.iter().any(|p| {
                        let required_segments =
                            p.split("::").filter(|s| !s.is_empty()).collect::<Vec<_>>();
                        required_segments.iter().all(|seg| f.path.contains(seg))
                    })
            })
            .and_then(|f| f.fn_def)
        {
            return found;
        }
    }
    let mut similar = deps
        .functions
        .iter()
        .filter(|f| {
            paths.iter().any(|p| {
                let last = p.rsplit("::").next().unwrap_or(p);
                f.path.contains(last)
            })
        })
        .map(|f| format!("{} (fn_def={})", f.path, f.fn_def.is_some()))
        .collect::<Vec<_>>();
    similar.sort();
    similar.truncate(40);
    let joined = paths.join(", ");
    panic!(
        "missing dependency function (any of): {joined}\nexample matches:\n{}",
        similar.join("\n")
    );
}

fn find_dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> Option<rustc_gen::FnDef> {
    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| f.path == path && f.fn_def.is_some())
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| {
            f.path.ends_with(path)
                && f.fn_def.is_some()
                && !f.path.contains("::{closure")
                && !f.path.contains("{{")
        })
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(last) = path.rsplit("::").next() {
        let required_segments = path
            .split("::")
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                f.path.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && !f.path.contains("::{closure")
                    && !f.path.contains("{{")
                    && required_segments.iter().all(|seg| f.path.contains(seg))
            })
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
    }

    None
}

pub(crate) fn dep_adt(deps: &rustc_gen::DependencyInfo, path: &str) -> rustc_gen::AdtDef {
    if let Some(found) = deps.types.iter().find(|t| t.path == path).map(|t| t.adt) {
        return found;
    }

    if let Some(found) = deps.types.iter().find(|t| t.path.ends_with(path)).map(|t| t.adt) {
        return found;
    }

    if let Some(last) = path.rsplit("::").next() {
        if let Some(found) = deps
            .types
            .iter()
            .find(|t| t.path.ends_with(&format!("::{last}")))
            .map(|t| t.adt)
        {
            return found;
        }
    }

    panic!("missing dependency type: {path}");
}

pub(crate) fn dep_adt_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> rustc_gen::AdtDef {
    for path in paths {
        if let Some(found) = deps.types.iter().find(|t| t.path == *path).map(|t| t.adt) {
            return found;
        }
        if let Some(found) = deps.types.iter().find(|t| t.path.ends_with(path)).map(|t| t.adt) {
            return found;
        }
    }
    panic!("missing dependency type (any of): {}", paths.join(", "));
}
