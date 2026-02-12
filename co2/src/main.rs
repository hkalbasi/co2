#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rustc_ast::{Attribute, MetaItemKind};
use rustc_driver::{Callbacks, Compilation};

use rustc_public_generative as gen;

use co2_hir_mir::{
    parse_and_lower, Callee as HirCallee, Function as HirFunction, LocalDecl as HirLocalDecl,
    MirModule, MirOp as HirOp, Operand as HirOperand, Type as HirType,
};
use itertools::Itertools;

fn is_language_co2(attr: &Attribute) -> bool {
    let meta = match attr.meta() {
        Some(meta) => meta,
        None => return false,
    };

    let path_segments = meta
        .path
        .segments
        .iter()
        .map(|s| s.ident.as_str())
        .collect::<Vec<_>>();

    if path_segments.as_slice() == ["language_co2"] {
        return true;
    }

    if path_segments.as_slice() == ["co2", "language"] {
        return true;
    }

    let is_language = meta
        .path
        .segments
        .iter()
        .map(|s| s.ident.as_str())
        .eq(std::iter::once("language"));

    if !is_language {
        return false;
    }

    match &meta.kind {
        MetaItemKind::List(items) => items.iter().any(|item| match item {
            rustc_ast::MetaItemInner::MetaItem(item) => {
                item.path.segments.iter().all(|s| s.ident.as_str() == "co2")
            }
            rustc_ast::MetaItemInner::Lit(_) => false,
        }),
        _ => false,
    }
}

static CO2_ENABLED: AtomicBool = AtomicBool::new(false);

struct DetectCallbacks {
    co2_file: Option<PathBuf>,
}

impl DetectCallbacks {
    fn new() -> Self {
        Self { co2_file: None }
    }
}

impl Callbacks for DetectCallbacks {
    fn after_crate_root_parsing(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        for attr in &krate.attrs {
            if is_language_co2(attr) {
                CO2_ENABLED.store(true, Ordering::Relaxed);
                let files_lock = compiler.sess.source_map().files();
                let original_file = files_lock.iter().exactly_one().unwrap();

                let rustc_span::FileName::Real(original_file) = &original_file.name else {
                    panic!("File was not real");
                };

                let original_file = original_file.path(rustc_span::RemapPathScopeComponents::MACRO);
                let co2_file = original_file.with_extension("co2");
                drop(files_lock);
                self.co2_file = Some(co2_file);
                return Compilation::Stop;
            }
        }

        Compilation::Continue
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut callbacks = DetectCallbacks::new();

    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut callbacks)
    });

    if !CO2_ENABLED.load(Ordering::Relaxed) {
        std::process::exit(exit_code);
    }

    let co2_file = callbacks.co2_file.expect("co2 file missing");
    if let Err(payload) = std::panic::catch_unwind(|| run_co2_compiler(co2_file)) {
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2 panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2 panic: {msg}");
        } else {
            eprintln!("co2 panic: non-string payload");
        }
        std::process::exit(101);
    }
}

fn run_co2_compiler(co2_file: PathBuf) {
    let co2_src = std::fs::read_to_string(&co2_file).expect("failed to read co2 file");
    let leaked: &'static str = Box::leak(co2_src.into_boxed_str());

    let module =
        parse_and_lower(co2_file.to_string_lossy().into_owned(), leaked)
            .expect("failed to parse and lower co2");
    let module_for_items = module.clone();

    let file_id_cell: Arc<Mutex<Option<gen::FileId>>> = Arc::new(Mutex::new(None));
    let co2_path = co2_file.clone();
    let co2_src_for_ctx = leaked;

    gen::generate(
        {
            let file_id_cell = file_id_cell.clone();
            move |ctx, deps| {
                let file_id = ctx.add_custom_file(&co2_path, co2_src_for_ctx);
                *file_id_cell.lock().unwrap() = Some(file_id);
                build_items(&module_for_items, deps)
            }
        },
        {
            let file_id_cell = file_id_cell.clone();
            move |ctx, deps, defined| {
                if std::env::var("CO2_DEBUG_DEFINED").is_ok() {
                    eprintln!(
                        "defined items: {}",
                        defined
                            .items
                            .iter()
                            .map(|i| i.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    eprintln!(
                        "module externs: {}",
                        module
                            .externs
                            .iter()
                            .map(|e| e.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                let file_id = file_id_cell
                    .lock()
                    .unwrap()
                    .expect("missing registered file");
                let mut bodies = Vec::new();
                for func in &module.functions {
                    let body = build_mir(func, &module, &deps, &defined, &ctx, file_id);
                    bodies.push(gen::ItemMirInfo {
                        id: func_item_id(func.name.as_str()),
                        body,
                    });
                }
                bodies
            }
        },
    );
}

fn func_item_id(name: &str) -> gen::ItemId {
    let mut hash = 0u64;
    for b in name.as_bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(*b as u64);
    }
    gen::ItemId::new(hash.max(1))
}

fn build_items(module: &MirModule, deps: gen::DependencyInfo) -> gen::CurrentCrateInfo {
    let mut items = Vec::new();
    let mut entry = None;

    for func in &module.functions {
        let id = func_item_id(&func.name);
        if func.name == "main" {
            entry = Some(id);
        }
        items.push(gen::ItemInfo {
            id,
            name: func.name.clone(),
            parent: None,
            kind: gen::ItemKind::Function,
        });
    }

    for ext in &module.externs {
        let id = func_item_id(&ext.name);
        items.push(gen::ItemInfo {
            id,
            name: ext.name.clone(),
            parent: None,
            kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                inputs: ext
                    .sig
                    .params
                    .iter()
                    .map(|t| mir_ty_from_type(t, Some(module), &deps))
                    .collect(),
                output: mir_ty_from_type(&ext.sig.ret, Some(module), &deps),
                abi: gen::FunctionAbi::C,
                is_unsafe: true,
            }),
        });
    }

    gen::CurrentCrateInfo {
        crate_name: "co2".to_owned(),
        entry,
        items,
    }
}

fn mir_ty_from_type(
    ty: &HirType,
    module: Option<&MirModule>,
    deps: &gen::DependencyInfo,
) -> gen::MirTy {
    match ty {
        HirType::Void => gen::MirTy::new_tuple(&[]),
        HirType::Int => gen::MirTy::signed_ty(gen::PublicIntTy::I32),
        HirType::Char => gen::MirTy::signed_ty(gen::PublicIntTy::I8),
        HirType::Ptr(inner) => {
            gen::MirTy::new_ptr(mir_ty_from_type(inner, module, deps), gen::MirMutability::Mut)
        }
        HirType::Array(inner) => {
            gen::MirTy::new_ptr(mir_ty_from_type(inner, module, deps), gen::MirMutability::Mut)
        }
        HirType::RustPath(path) => mir_ty_from_rust_path(path, module, deps),
    }
}

fn mir_ty_from_rust_path(
    path: &co2_parser::RustPath,
    module: Option<&MirModule>,
    deps: &gen::DependencyInfo,
) -> gen::MirTy {
    let base = rust_path_base_string(path);
    if let Some(prim) = primitive_mir_ty(&base) {
        return prim;
    }

    let adt = dep_adt(deps, &base);
    let mut generic_args = rust_path_generic_args(path)
        .into_iter()
        .map(|arg| {
            gen::GenericArgKind::Type(mir_ty_from_rust_path(&arg, module, deps))
        })
        .collect::<Vec<_>>();
    if (base == "std::vec::Vec" || base == "alloc::vec::Vec" || base.ends_with("::Vec"))
        && generic_args.len() == 1
    {
        let global = dep_adt_any(deps, &["alloc::alloc::Global", "std::alloc::Global"]);
        generic_args.push(gen::GenericArgKind::Type(gen::MirTy::from_rigid_kind(
            gen::RigidTy::Adt(global, gen::GenericArgs(vec![])),
        )));
    }
    gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(adt, gen::GenericArgs(generic_args)))
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

fn rust_path_generic_args(path: &co2_parser::RustPath) -> Vec<co2_parser::RustPath> {
    for seg in &path.segments {
        if let co2_parser::RustPathSegment::Generics(args) = &seg.0 {
            return args.iter().map(|arg| arg.0.clone()).collect();
        }
    }
    Vec::new()
}

fn primitive_mir_ty(name: &str) -> Option<gen::MirTy> {
    match name {
        "u8" => Some(gen::MirTy::unsigned_ty(gen::PublicUintTy::U8)),
        "i8" => Some(gen::MirTy::signed_ty(gen::PublicIntTy::I8)),
        "u32" => Some(gen::MirTy::unsigned_ty(gen::PublicUintTy::U32)),
        "i32" => Some(gen::MirTy::signed_ty(gen::PublicIntTy::I32)),
        "usize" => Some(gen::MirTy::usize_ty()),
        "isize" => Some(gen::MirTy::signed_ty(gen::PublicIntTy::Isize)),
        _ => None,
    }
}

fn dep_fn(deps: &gen::DependencyInfo, path: &str) -> gen::FnDef {
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

fn dep_fn_any(deps: &gen::DependencyInfo, paths: &[&str]) -> gen::FnDef {
    for path in paths {
        if let Some(found) = find_dep_fn(deps, path) {
            return found;
        }
    }
    if let Some(last) = paths
        .iter()
        .find_map(|p| p.rsplit("::").next())
    {
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                f.path.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && paths.iter().any(|p| {
                        let required_segments = p
                            .split("::")
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>();
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

fn find_dep_fn(deps: &gen::DependencyInfo, path: &str) -> Option<gen::FnDef> {
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

fn dep_adt(deps: &gen::DependencyInfo, path: &str) -> gen::AdtDef {
    if let Some(found) = deps.types.iter().find(|t| t.path == path).map(|t| t.adt) {
        return found;
    }

    if let Some(found) = deps
        .types
        .iter()
        .find(|t| t.path.ends_with(path))
        .map(|t| t.adt)
    {
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

fn dep_adt_any(deps: &gen::DependencyInfo, paths: &[&str]) -> gen::AdtDef {
    for path in paths {
        if let Some(found) = deps.types.iter().find(|t| t.path == *path).map(|t| t.adt) {
            return found;
        }
        if let Some(found) = deps
            .types
            .iter()
            .find(|t| t.path.ends_with(path))
            .map(|t| t.adt)
        {
            return found;
        }
    }
    panic!("missing dependency type (any of): {}", paths.join(", "));
}

fn build_mir(
    func: &HirFunction,
    module: &MirModule,
    deps: &gen::DependencyInfo,
    defined: &gen::DefinedCrateInfo,
    ctx: &gen::Context,
    file_id: gen::FileId,
) -> gen::MirBody {
    let span = ctx.span_in_file(file_id, 0, 0);

    let mut locals = Vec::new();
    for local in &func.locals {
        locals.push(gen::MirLocalDecl {
            ty: mir_ty_from_type(&local.ty, Some(module), deps),
            span,
            mutability: gen::MirMutability::Mut,
        });
    }

    let mut blocks = Vec::new();
    let mut stmts = Vec::new();

    let mut extra_locals: Vec<gen::MirLocalDecl> = Vec::new();

    for op in &func.ops {
        match op {
            HirOp::Assign { dst, src } => {
                let rvalue = gen::MirRvalue::Use(lower_operand(
                    src,
                    &locals,
                    &mut extra_locals,
                    deps,
                    module,
                    &mut blocks,
                    &mut stmts,
                    ctx,
                    file_id,
                ));
                stmts.push(gen::MirStatement {
                    kind: gen::MirStatementKind::Assign(place(*dst), rvalue),
                    span,
                });
            }
            HirOp::Call { func: callee, args, dest } => {
                let (func_op, arg_ops) = lower_call(
                    callee,
                    args,
                    &func.locals,
                    &locals,
                    &mut extra_locals,
                    deps,
                    module,
                    &mut blocks,
                    &mut stmts,
                    ctx,
                    file_id,
                    span,
                    defined,
                );
                let dest_place = dest
                    .map(|d| place(d))
                    .unwrap_or_else(|| {
                        let idx = locals.len() + extra_locals.len();
                        let ret_ty = call_return_ty(callee, module, deps);
                        extra_locals.push(gen::MirLocalDecl {
                            ty: ret_ty,
                            span,
                            mutability: gen::MirMutability::Mut,
                        });
                        place(idx)
                    });
                emit_call_block(
                    &mut blocks,
                    &mut stmts,
                    span,
                    func_op,
                    arg_ops,
                    dest_place,
                );
            }
            HirOp::Return => {
                blocks.push(gen::MirBasicBlock {
                    statements: std::mem::take(&mut stmts),
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Return,
                        span,
                    },
                });
            }
        }
    }

    if !stmts.is_empty() {
        blocks.push(gen::MirBasicBlock {
            statements: std::mem::take(&mut stmts),
            terminator: gen::MirTerminator {
                kind: gen::MirTerminatorKind::Return,
                span,
            },
        });
    }

    locals.extend(extra_locals);

    gen::MirBody::new(blocks, locals, 0, vec![], None, span)
}

fn place(local: usize) -> gen::MirPlace {
    gen::MirPlace {
        local,
        projection: vec![],
    }
}

fn emit_call_block(
    blocks: &mut Vec<gen::MirBasicBlock>,
    stmts: &mut Vec<gen::MirStatement>,
    span: gen::PublicSpan,
    func: gen::MirOperand,
    args: Vec<gen::MirOperand>,
    destination: gen::MirPlace,
) {
    let next = blocks.len() + 1;
    blocks.push(gen::MirBasicBlock {
        statements: std::mem::take(stmts),
        terminator: gen::MirTerminator {
            kind: gen::MirTerminatorKind::Call {
                func,
                args,
                destination,
                target: Some(next),
                unwind: gen::MirUnwindAction::Continue,
            },
            span,
        },
    });
}

fn infer_generic_args_for_call(
    path: &str,
    args: &[HirOperand],
    hir_locals: &[HirLocalDecl],
    module: &MirModule,
    deps: &gen::DependencyInfo,
) -> Vec<gen::GenericArgKind> {
    if path.ends_with("::Option::unwrap") || path.ends_with("::option::Option::unwrap") {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return generic_args_from_type(arg_ty, module, deps);
        }
    }

    if path.ends_with("::Result::unwrap") || path.ends_with("::result::Result::unwrap") {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return generic_args_from_type(arg_ty, module, deps);
        }
    }

    if path.ends_with("::Iterator::nth") || path.ends_with("::Iterator::next") {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return vec![gen::GenericArgKind::Type(mir_ty_from_type(
                arg_ty,
                Some(module),
                deps,
            ))];
        }
    }

    if path.ends_with("::CString::new") {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return vec![gen::GenericArgKind::Type(mir_ty_from_type(
                arg_ty,
                Some(module),
                deps,
            ))];
        }
    }

    if path.ends_with("::fs::read") || path.ends_with("::std::fs::read") {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return vec![gen::GenericArgKind::Type(mir_ty_from_type(
                arg_ty,
                Some(module),
                deps,
            ))];
        }
    }

    if path.ends_with("::Vec::as_ptr")
        || path.ends_with("::Vec::as_mut_ptr")
        || path.ends_with("::Vec::len")
    {
        if let Some(arg_ty) = hir_operand_type(args.get(0), hir_locals) {
            return vec_method_generic_args(arg_ty, module, deps);
        }
    }

    Vec::new()
}

fn hir_operand_type<'a>(
    operand: Option<&'a HirOperand>,
    hir_locals: &'a [HirLocalDecl],
) -> Option<&'a HirType> {
    let HirOperand::Local(local) = operand? else {
        return None;
    };
    hir_locals.get(*local).map(|l| &l.ty)
}

fn generic_args_from_type(
    ty: &HirType,
    module: &MirModule,
    deps: &gen::DependencyInfo,
) -> Vec<gen::GenericArgKind> {
    let HirType::RustPath(path) = ty else {
        return Vec::new();
    };
    rust_path_generic_args(path)
        .into_iter()
        .map(|arg| gen::GenericArgKind::Type(mir_ty_from_rust_path(&arg, Some(module), deps)))
        .collect()
}

fn vec_method_generic_args(
    ty: &HirType,
    module: &MirModule,
    deps: &gen::DependencyInfo,
) -> Vec<gen::GenericArgKind> {
    let mut args = generic_args_from_type(ty, module, deps);
    if args.len() == 1 {
        let global = dep_adt_any(deps, &["alloc::alloc::Global", "std::alloc::Global"]);
        let global_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
            global,
            gen::GenericArgs(vec![]),
        ));
        args.push(gen::GenericArgKind::Type(global_ty));
    }
    args
}

enum AutorefKind {
    Shared,
    Mut,
}

fn autoref_kind_for_path(path: &str) -> Option<AutorefKind> {
    if path.ends_with("::Iterator::nth") || path.ends_with("::Iterator::next") {
        return Some(AutorefKind::Mut);
    }

    if path.ends_with("::CString::as_ptr")
        || path.ends_with("::String::as_ptr")
        || path.ends_with("::String::as_str")
        || path.ends_with("::Vec::as_ptr")
        || path.ends_with("::Vec::len")
    {
        return Some(AutorefKind::Shared);
    }

    if path.ends_with("::Vec::as_mut_ptr") {
        return Some(AutorefKind::Mut);
    }

    None
}

fn autoref_call_arg(
    path: &str,
    arg_index: usize,
    arg: &HirOperand,
    hir_locals: &[HirLocalDecl],
    locals: &[gen::MirLocalDecl],
    extra_locals: &mut Vec<gen::MirLocalDecl>,
    module: &MirModule,
    deps: &gen::DependencyInfo,
    stmts: &mut Vec<gen::MirStatement>,
    span: gen::PublicSpan,
) -> Option<gen::MirOperand> {
    if arg_index != 0 {
        return None;
    }
    let kind = autoref_kind_for_path(path)?;
    let HirOperand::Local(local) = arg else {
        return None;
    };
    let hir_ty = &hir_locals.get(*local)?.ty;
    let base_ty = mir_ty_from_type(hir_ty, Some(module), deps);
    let (ref_ty, borrow_kind) = match kind {
        AutorefKind::Shared => (
            gen::MirTy::new_ref(
                gen::Region {
                    kind: gen::RegionKind::ReErased,
                },
                base_ty,
                gen::MirMutability::Not,
            ),
            gen::MirBorrowKind::Shared,
        ),
        AutorefKind::Mut => (
            gen::MirTy::new_ref(
                gen::Region {
                    kind: gen::RegionKind::ReErased,
                },
                base_ty,
                gen::MirMutability::Mut,
            ),
            gen::MirBorrowKind::Mut {
                kind: gen::MirMutBorrowKind::Default,
            },
        ),
    };

    let ref_local = locals.len() + extra_locals.len();
    extra_locals.push(gen::MirLocalDecl {
        ty: ref_ty,
        span,
        mutability: gen::MirMutability::Not,
    });
    stmts.push(gen::MirStatement {
        kind: gen::MirStatementKind::Assign(
            place(ref_local),
            gen::MirRvalue::Ref(
                gen::Region {
                    kind: gen::RegionKind::ReErased,
                },
                borrow_kind,
                place(*local),
            ),
        ),
        span,
    });
    Some(gen::MirOperand::Move(place(ref_local)))
}

fn lower_call(
    callee: &HirCallee,
    args: &[HirOperand],
    hir_locals: &[HirLocalDecl],
    locals: &[gen::MirLocalDecl],
    extra_locals: &mut Vec<gen::MirLocalDecl>,
    deps: &gen::DependencyInfo,
    module: &MirModule,
    blocks: &mut Vec<gen::MirBasicBlock>,
    stmts: &mut Vec<gen::MirStatement>,
    ctx: &gen::Context,
    file_id: gen::FileId,
    span: gen::PublicSpan,
    defined: &gen::DefinedCrateInfo,
) -> (gen::MirOperand, Vec<gen::MirOperand>) {
    let (func_def, path) = match callee {
        HirCallee::Path(path) => {
            if let Some(item) = defined.items.iter().find(|i| i.name == *path) {
                if std::env::var("CO2_DEBUG_CALLS").is_ok() {
                    eprintln!("call {path}: resolved as defined item");
                }
                (item.fn_def().expect("missing fn def"), path.as_str())
            } else {
                if std::env::var("CO2_DEBUG_CALLS").is_ok() {
                    eprintln!("call {path}: resolving from deps");
                }
                (resolve_dep_fn_for_path(deps, path), path.as_str())
            }
        }
    };

    let generic_args = infer_generic_args_for_call(path, args, hir_locals, module, deps);
    let func_op = fn_const_operand(func_def, generic_args, span);
    let mut arg_ops = Vec::new();
    for (idx, arg) in args.iter().enumerate() {
        if path.ends_with("::Iterator::nth") && idx == 1 {
            if let HirOperand::ConstInt(v, sp) = arg {
                let arg_span = ctx.span_in_file(file_id, sp.start as u32, sp.end as u32);
                let c = gen::PublicMirConst::try_from_uint(*v as u128, gen::PublicUintTy::Usize)
                    .expect("failed to build usize const");
                arg_ops.push(gen::MirOperand::Constant(gen::MirConst {
                    span: arg_span,
                    user_ty: None,
                    const_: c,
                }));
                continue;
            }
        }
        if let Some(auto_ref) = autoref_call_arg(
            path,
            idx,
            arg,
            hir_locals,
            locals,
            extra_locals,
            module,
            deps,
            stmts,
            span,
        ) {
            arg_ops.push(auto_ref);
            continue;
        }
        if let HirOperand::Local(local) = arg {
            arg_ops.push(gen::MirOperand::Move(place(*local)));
            continue;
        }
        arg_ops.push(lower_operand(
            arg,
            locals,
            extra_locals,
            deps,
            module,
            blocks,
            stmts,
            ctx,
            file_id,
        ));
    }

    (func_op, arg_ops)
}

fn resolve_dep_fn_for_path(deps: &gen::DependencyInfo, path: &str) -> gen::FnDef {
    if path.ends_with("::Option::unwrap") || path.ends_with("::option::Option::unwrap") {
        return dep_fn_any(
            deps,
            &["core::option::Option::unwrap", "std::option::Option::unwrap"],
        );
    }
    if path.ends_with("::Result::unwrap") || path.ends_with("::result::Result::unwrap") {
        return dep_fn_any(
            deps,
            &["core::result::Result::unwrap", "std::result::Result::unwrap"],
        );
    }
    if path.ends_with("::Vec::as_mut_ptr") {
        return dep_fn_any(
            deps,
            &["alloc::vec::Vec::as_mut_ptr", "std::vec::Vec::as_mut_ptr"],
        );
    }
    if path.ends_with("::Vec::as_ptr") {
        return dep_fn_any(deps, &["alloc::vec::Vec::as_ptr", "std::vec::Vec::as_ptr"]);
    }
    if path.ends_with("::Vec::len") {
        return dep_fn_any(deps, &["alloc::vec::Vec::len", "std::vec::Vec::len"]);
    }
    dep_fn(deps, path)
}

fn call_return_ty(
    callee: &HirCallee,
    module: &MirModule,
    deps: &gen::DependencyInfo,
) -> gen::MirTy {
    match callee {
        HirCallee::Path(path) => {
            if let Some(f) = module.functions.iter().find(|f| f.name == *path) {
                return mir_ty_from_type(&f.sig.ret, Some(module), deps);
            }
            if let Some(f) = module.externs.iter().find(|f| f.name == *path) {
                return mir_ty_from_type(&f.sig.ret, Some(module), deps);
            }
            gen::MirTy::new_tuple(&[])
        }
    }
}

fn lower_operand(
    operand: &HirOperand,
    locals: &[gen::MirLocalDecl],
    extra_locals: &mut Vec<gen::MirLocalDecl>,
    deps: &gen::DependencyInfo,
    _module: &MirModule,
    blocks: &mut Vec<gen::MirBasicBlock>,
    stmts: &mut Vec<gen::MirStatement>,
    ctx: &gen::Context,
    file_id: gen::FileId,
) -> gen::MirOperand {
    match operand {
        HirOperand::Local(l) => gen::MirOperand::Copy(place(*l)),
        HirOperand::ConstInt(v, sp) => {
            let span = ctx.span_in_file(file_id, sp.start as u32, sp.end as u32);
            let c = gen::PublicMirConst::try_from_uint(*v as u128, gen::PublicUintTy::U32)
                .expect("failed to build int const");
            let const_op = gen::MirOperand::Constant(gen::MirConst {
                span,
                user_ty: None,
                const_: c,
            });
            let tmp_local = locals.len() + extra_locals.len();
            let i32_ty = gen::MirTy::signed_ty(gen::PublicIntTy::I32);
            extra_locals.push(gen::MirLocalDecl {
                ty: i32_ty,
                span,
                mutability: gen::MirMutability::Mut,
            });
            stmts.push(gen::MirStatement {
                kind: gen::MirStatementKind::Assign(
                    place(tmp_local),
                    gen::MirRvalue::Cast(gen::MirCastKind::IntToInt, const_op, i32_ty),
                ),
                span,
            });
            gen::MirOperand::Copy(place(tmp_local))
        }
        HirOperand::ConstStr(s, sp) => {
            let span = ctx.span_in_file(file_id, sp.start as u32, sp.end as u32);
            let mut value = s.clone();
            if !value.ends_with('\0') {
                value.push('\0');
            }
            let str_const = gen::MirOperand::Constant(gen::MirConst {
                span,
                user_ty: None,
                const_: gen::PublicMirConst::from_str(&value),
            });
            let as_ptr = dep_fn_any(deps, &["core::str::as_ptr", "std::str::as_ptr"]);
            let u8_ty = gen::MirTy::unsigned_ty(gen::PublicUintTy::U8);
            let ptr_u8_ty = gen::MirTy::new_ptr(u8_ty, gen::MirMutability::Not);
            let ptr_u8_local = locals.len() + extra_locals.len();
            extra_locals.push(gen::MirLocalDecl {
                ty: ptr_u8_ty,
                span,
                mutability: gen::MirMutability::Mut,
            });
            emit_call_block(
                blocks,
                stmts,
                span,
                fn_const_operand(as_ptr, vec![], span),
                vec![str_const],
                place(ptr_u8_local),
            );

            let i8_ty = gen::MirTy::signed_ty(gen::PublicIntTy::I8);
            let ptr_i8_ty = gen::MirTy::new_ptr(i8_ty, gen::MirMutability::Mut);
            let ptr_i8_local = locals.len() + extra_locals.len();
            extra_locals.push(gen::MirLocalDecl {
                ty: ptr_i8_ty,
                span,
                mutability: gen::MirMutability::Mut,
            });
            stmts.push(gen::MirStatement {
                kind: gen::MirStatementKind::Assign(
                    place(ptr_i8_local),
                    gen::MirRvalue::Cast(
                        gen::MirCastKind::PtrToPtr,
                        gen::MirOperand::Copy(place(ptr_u8_local)),
                        ptr_i8_ty,
                    ),
                ),
                span,
            });

            gen::MirOperand::Copy(place(ptr_i8_local))
        }
    }
}

fn fn_const_operand(
    fn_def: gen::FnDef,
    generic_args: Vec<gen::GenericArgKind>,
    span: gen::PublicSpan,
) -> gen::MirOperand {
    let fn_ty =
        gen::MirTy::from_rigid_kind(gen::RigidTy::FnDef(fn_def, gen::GenericArgs(generic_args)));
    let c = gen::PublicMirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    gen::MirOperand::Constant(gen::MirConst {
        span,
        user_ty: None,
        const_: c,
    })
}
