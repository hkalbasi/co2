use co2_hir_mir::{
    Callee as HirCallee, Function as HirFunction, LocalDecl as HirLocalDecl, MirModule,
    MirOp as HirOp, Operand as HirOperand, Type as HirType,
};
use rustc_public_generative as rustc_gen;

use crate::types::{
    CompileMode, dep_adt_any, dep_fn, dep_fn_any, func_item_id, mir_ty_from_rust_path,
    mir_ty_from_type, rust_path_generic_args,
};

pub(crate) fn build_mir(
    func: &HirFunction,
    module: &MirModule,
    deps: &rustc_gen::DependencyInfo,
    defined: &rustc_gen::DefinedCrateInfo,
    ctx: &rustc_gen::Context,
    file_id: rustc_gen::FileId,
    mode: CompileMode,
) -> rustc_gen::MirBody {
    let span = ctx.span_in_file(file_id, 0, 0);

    let mut locals = Vec::new();
    for local in &func.locals {
        locals.push(rustc_gen::MirLocalDecl {
            ty: mir_ty_from_type(&local.ty, Some(module), deps),
            span,
            mutability: rustc_gen::MirMutability::Mut,
        });
    }

    let mut blocks = Vec::new();
    let mut stmts = Vec::new();

    let mut extra_locals: Vec<rustc_gen::MirLocalDecl> = Vec::new();

    for op in &func.ops {
        match op {
            HirOp::Assign { dst, src } => {
                let rvalue = rustc_gen::MirRvalue::Use(lower_operand(
                    src,
                    &locals,
                    &mut extra_locals,
                    deps,
                    module,
                    &mut blocks,
                    &mut stmts,
                    ctx,
                    file_id,
                    mode,
                ));
                stmts.push(rustc_gen::MirStatement {
                    kind: rustc_gen::MirStatementKind::Assign(place(*dst), rvalue),
                    span,
                });
            }
            HirOp::Call {
                func: callee,
                args,
                dest,
            } => {
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
                    mode,
                );
                let dest_place = dest.map(place).unwrap_or_else(|| {
                    let idx = locals.len() + extra_locals.len();
                    let ret_ty = call_return_ty(callee, module, deps);
                    extra_locals.push(rustc_gen::MirLocalDecl {
                        ty: ret_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    });
                    place(idx)
                });
                emit_call_block(&mut blocks, &mut stmts, span, func_op, arg_ops, dest_place);
            }
            HirOp::Return => {
                blocks.push(rustc_gen::MirBasicBlock {
                    statements: std::mem::take(&mut stmts),
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Return,
                        span,
                    },
                });
            }
        }
    }

    if !stmts.is_empty() {
        blocks.push(rustc_gen::MirBasicBlock {
            statements: std::mem::take(&mut stmts),
            terminator: rustc_gen::MirTerminator {
                kind: rustc_gen::MirTerminatorKind::Return,
                span,
            },
        });
    }

    locals.extend(extra_locals);

    rustc_gen::MirBody::new(blocks, locals, 0, vec![], None, span)
}

fn place(local: usize) -> rustc_gen::MirPlace {
    rustc_gen::MirPlace {
        local,
        projection: vec![],
    }
}

fn emit_call_block(
    blocks: &mut Vec<rustc_gen::MirBasicBlock>,
    stmts: &mut Vec<rustc_gen::MirStatement>,
    span: rustc_gen::PublicSpan,
    func: rustc_gen::MirOperand,
    args: Vec<rustc_gen::MirOperand>,
    destination: rustc_gen::MirPlace,
) {
    let next = blocks.len() + 1;
    blocks.push(rustc_gen::MirBasicBlock {
        statements: std::mem::take(stmts),
        terminator: rustc_gen::MirTerminator {
            kind: rustc_gen::MirTerminatorKind::Call {
                func,
                args,
                destination,
                target: Some(next),
                unwind: rustc_gen::MirUnwindAction::Continue,
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
    deps: &rustc_gen::DependencyInfo,
) -> Vec<rustc_gen::GenericArgKind> {
    if path.ends_with("::Option::unwrap") || path.ends_with("::option::Option::unwrap") {
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
            return generic_args_from_type(arg_ty, module, deps);
        }
    }

    if path.ends_with("::Result::unwrap") || path.ends_with("::result::Result::unwrap") {
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
            return generic_args_from_type(arg_ty, module, deps);
        }
    }

    if path.ends_with("::Iterator::nth") || path.ends_with("::Iterator::next") {
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
            return vec![rustc_gen::GenericArgKind::Type(mir_ty_from_type(
                arg_ty,
                Some(module),
                deps,
            ))];
        }
    }

    if path.ends_with("::CString::new") {
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
            return vec![rustc_gen::GenericArgKind::Type(mir_ty_from_type(
                arg_ty,
                Some(module),
                deps,
            ))];
        }
    }

    if path.ends_with("::fs::read") || path.ends_with("::std::fs::read") {
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
            return vec![rustc_gen::GenericArgKind::Type(mir_ty_from_type(
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
        if let Some(arg_ty) = hir_operand_type(args.first(), hir_locals) {
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
    deps: &rustc_gen::DependencyInfo,
) -> Vec<rustc_gen::GenericArgKind> {
    let HirType::RustPath(path) = ty else {
        return Vec::new();
    };
    rust_path_generic_args(path)
        .into_iter()
        .map(|arg| rustc_gen::GenericArgKind::Type(mir_ty_from_rust_path(&arg, Some(module), deps)))
        .collect()
}

fn vec_method_generic_args(
    ty: &HirType,
    module: &MirModule,
    deps: &rustc_gen::DependencyInfo,
) -> Vec<rustc_gen::GenericArgKind> {
    let mut args = generic_args_from_type(ty, module, deps);
    if args.len() == 1 {
        let global = dep_adt_any(deps, &["alloc::alloc::Global", "std::alloc::Global"]);
        let global_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
            global,
            rustc_gen::GenericArgs(vec![]),
        ));
        args.push(rustc_gen::GenericArgKind::Type(global_ty));
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

#[allow(clippy::too_many_arguments)]
fn autoref_call_arg(
    path: &str,
    arg_index: usize,
    arg: &HirOperand,
    hir_locals: &[HirLocalDecl],
    locals: &[rustc_gen::MirLocalDecl],
    extra_locals: &mut Vec<rustc_gen::MirLocalDecl>,
    module: &MirModule,
    deps: &rustc_gen::DependencyInfo,
    stmts: &mut Vec<rustc_gen::MirStatement>,
    span: rustc_gen::PublicSpan,
) -> Option<rustc_gen::MirOperand> {
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
            rustc_gen::MirTy::new_ref(
                rustc_gen::Region {
                    kind: rustc_gen::RegionKind::ReErased,
                },
                base_ty,
                rustc_gen::MirMutability::Not,
            ),
            rustc_gen::MirBorrowKind::Shared,
        ),
        AutorefKind::Mut => (
            rustc_gen::MirTy::new_ref(
                rustc_gen::Region {
                    kind: rustc_gen::RegionKind::ReErased,
                },
                base_ty,
                rustc_gen::MirMutability::Mut,
            ),
            rustc_gen::MirBorrowKind::Mut {
                kind: rustc_gen::MirMutBorrowKind::Default,
            },
        ),
    };

    let ref_local = locals.len() + extra_locals.len();
    extra_locals.push(rustc_gen::MirLocalDecl {
        ty: ref_ty,
        span,
        mutability: rustc_gen::MirMutability::Not,
    });
    stmts.push(rustc_gen::MirStatement {
        kind: rustc_gen::MirStatementKind::Assign(
            place(ref_local),
            rustc_gen::MirRvalue::Ref(
                rustc_gen::Region {
                    kind: rustc_gen::RegionKind::ReErased,
                },
                borrow_kind,
                place(*local),
            ),
        ),
        span,
    });
    Some(rustc_gen::MirOperand::Move(place(ref_local)))
}

#[allow(clippy::too_many_arguments)]
fn lower_call(
    callee: &HirCallee,
    args: &[HirOperand],
    hir_locals: &[HirLocalDecl],
    locals: &[rustc_gen::MirLocalDecl],
    extra_locals: &mut Vec<rustc_gen::MirLocalDecl>,
    deps: &rustc_gen::DependencyInfo,
    module: &MirModule,
    blocks: &mut Vec<rustc_gen::MirBasicBlock>,
    stmts: &mut Vec<rustc_gen::MirStatement>,
    ctx: &rustc_gen::Context,
    file_id: rustc_gen::FileId,
    span: rustc_gen::PublicSpan,
    defined: &rustc_gen::DefinedCrateInfo,
    mode: CompileMode,
) -> (rustc_gen::MirOperand, Vec<rustc_gen::MirOperand>) {
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
                let c = rustc_gen::PublicMirConst::try_from_uint(
                    *v as u128,
                    rustc_gen::PublicUintTy::Usize,
                )
                .expect("failed to build usize const");
                arg_ops.push(rustc_gen::MirOperand::Constant(rustc_gen::MirConst {
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
            arg_ops.push(rustc_gen::MirOperand::Move(place(*local)));
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
            mode,
        ));
    }

    (func_op, arg_ops)
}

fn resolve_dep_fn_for_path(deps: &rustc_gen::DependencyInfo, path: &str) -> rustc_gen::FnDef {
    if path == "printf" || path.ends_with("::printf") {
        return dep_fn_any(deps, &["libc::printf", "libc::unix::printf"]);
    }
    if path.ends_with("::Option::unwrap") || path.ends_with("::option::Option::unwrap") {
        return dep_fn_any(
            deps,
            &[
                "core::option::Option::unwrap",
                "std::option::Option::unwrap",
            ],
        );
    }
    if path.ends_with("::Result::unwrap") || path.ends_with("::result::Result::unwrap") {
        return dep_fn_any(
            deps,
            &[
                "core::result::Result::unwrap",
                "std::result::Result::unwrap",
            ],
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
    deps: &rustc_gen::DependencyInfo,
) -> rustc_gen::MirTy {
    match callee {
        HirCallee::Path(path) => {
            if path == "printf" || path.ends_with("::printf") {
                return rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I32);
            }
            if let Some(f) = module.functions.iter().find(|f| f.name == *path) {
                return mir_ty_from_type(&f.sig.ret, Some(module), deps);
            }
            if let Some(f) = module.externs.iter().find(|f| f.name == *path) {
                return mir_ty_from_type(&f.sig.ret, Some(module), deps);
            }
            rustc_gen::MirTy::new_tuple(&[])
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_operand(
    operand: &HirOperand,
    locals: &[rustc_gen::MirLocalDecl],
    extra_locals: &mut Vec<rustc_gen::MirLocalDecl>,
    deps: &rustc_gen::DependencyInfo,
    _module: &MirModule,
    blocks: &mut Vec<rustc_gen::MirBasicBlock>,
    stmts: &mut Vec<rustc_gen::MirStatement>,
    ctx: &rustc_gen::Context,
    file_id: rustc_gen::FileId,
    mode: CompileMode,
) -> rustc_gen::MirOperand {
    match operand {
        HirOperand::Local(l) => rustc_gen::MirOperand::Copy(place(*l)),
        HirOperand::ConstInt(v, sp) => {
            let span = ctx.span_in_file(file_id, sp.start as u32, sp.end as u32);
            let c =
                rustc_gen::PublicMirConst::try_from_uint(*v as u128, rustc_gen::PublicUintTy::U32)
                    .expect("failed to build int const");
            let const_op = rustc_gen::MirOperand::Constant(rustc_gen::MirConst {
                span,
                user_ty: None,
                const_: c,
            });
            let tmp_local = locals.len() + extra_locals.len();
            let i32_ty = rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I32);
            extra_locals.push(rustc_gen::MirLocalDecl {
                ty: i32_ty,
                span,
                mutability: rustc_gen::MirMutability::Mut,
            });
            stmts.push(rustc_gen::MirStatement {
                kind: rustc_gen::MirStatementKind::Assign(
                    place(tmp_local),
                    rustc_gen::MirRvalue::Cast(rustc_gen::MirCastKind::IntToInt, const_op, i32_ty),
                ),
                span,
            });
            rustc_gen::MirOperand::Copy(place(tmp_local))
        }
        HirOperand::ConstStr(s, sp) => {
            let span = ctx.span_in_file(file_id, sp.start as u32, sp.end as u32);
            let mut value = s.clone();
            if !value.ends_with('\0') {
                value.push('\0');
            }
            let str_const = rustc_gen::MirOperand::Constant(rustc_gen::MirConst {
                span,
                user_ty: None,
                const_: rustc_gen::PublicMirConst::from_str(&value),
            });
            let as_ptr = dep_fn_any(deps, &["core::str::as_ptr", "std::str::as_ptr"]);
            let u8_ty = rustc_gen::MirTy::unsigned_ty(rustc_gen::PublicUintTy::U8);
            let ptr_u8_ty = rustc_gen::MirTy::new_ptr(u8_ty, rustc_gen::MirMutability::Not);
            let ptr_u8_local = locals.len() + extra_locals.len();
            extra_locals.push(rustc_gen::MirLocalDecl {
                ty: ptr_u8_ty,
                span,
                mutability: rustc_gen::MirMutability::Mut,
            });
            emit_call_block(
                blocks,
                stmts,
                span,
                fn_const_operand(as_ptr, vec![], span),
                vec![str_const],
                place(ptr_u8_local),
            );

            let i8_ty = rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I8);
            let ptr_i8_ty = rustc_gen::MirTy::new_ptr(
                i8_ty,
                if mode.no_main {
                    rustc_gen::MirMutability::Not
                } else {
                    rustc_gen::MirMutability::Mut
                },
            );
            let ptr_i8_local = locals.len() + extra_locals.len();
            extra_locals.push(rustc_gen::MirLocalDecl {
                ty: ptr_i8_ty,
                span,
                mutability: rustc_gen::MirMutability::Mut,
            });
            stmts.push(rustc_gen::MirStatement {
                kind: rustc_gen::MirStatementKind::Assign(
                    place(ptr_i8_local),
                    rustc_gen::MirRvalue::Cast(
                        rustc_gen::MirCastKind::PtrToPtr,
                        rustc_gen::MirOperand::Copy(place(ptr_u8_local)),
                        ptr_i8_ty,
                    ),
                ),
                span,
            });

            rustc_gen::MirOperand::Copy(place(ptr_i8_local))
        }
    }
}

fn fn_const_operand(
    fn_def: rustc_gen::FnDef,
    generic_args: Vec<rustc_gen::GenericArgKind>,
    span: rustc_gen::PublicSpan,
) -> rustc_gen::MirOperand {
    let fn_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::FnDef(
        fn_def,
        rustc_gen::GenericArgs(generic_args),
    ));
    let c = rustc_gen::PublicMirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    rustc_gen::MirOperand::Constant(rustc_gen::MirConst {
        span,
        user_ty: None,
        const_: c,
    })
}

pub(crate) fn build_item_mir_infos(
    module: &MirModule,
    deps: &rustc_gen::DependencyInfo,
    defined: &rustc_gen::DefinedCrateInfo,
    ctx: &rustc_gen::Context,
    file_id: rustc_gen::FileId,
    mode: CompileMode,
) -> Vec<rustc_gen::ItemMirInfo> {
    module
        .functions
        .iter()
        .map(|func| {
            let body = build_mir(func, module, deps, defined, ctx, file_id, mode);
            rustc_gen::ItemMirInfo {
                id: func_item_id(func.name.as_str()),
                body,
            }
        })
        .collect()
}
