#![feature(rustc_private)]

use std::collections::HashMap;

use rustc_public_generative as rustc_gen;
use rustc_public_generative::rustc_public::{
    mir::{
        AggregateKind, BasicBlock as MirBasicBlock, BinOp as MirBinOp, Body, BorrowKind, CastKind,
        ConstOperand, LocalDecl as MirLocalDecl, MutBorrowKind, Mutability, Operand as MirOperand,
        Place as MirPlace, ProjectionElem as MirProjection, Rvalue, Statement as MirStatement,
        StatementKind as MirStatementKind, Terminator as MirTerminator, TerminatorKind,
        UnwindAction,
    },
    ty::{
        FnDef, GenericArgKind, GenericArgs, IntTy, MirConst, Region, RegionKind, RigidTy,
        Span as RustSpan, Ty, TyKind, UintTy, VariantIdx,
    },
};

use co2_hir::{HirBinOp, HirBody, HirDecl, HirExpr, HirExprKind, HirStmt, LocalId, ResolvedValue};

pub fn build_mir_for_body(
    body: &HirBody,
    deps: &rustc_gen::DependencyInfo,
    ctx: &rustc_gen::HirStructureCtx,
    file_id: rustc_gen::FileId,
    is_rust_entry_main: bool,
) -> Body {
    let span = ctx.span_in_file(file_id, 0, 0);
    let exit_fn = if is_rust_entry_main {
        Some(dep_fn_any(
            deps,
            &["std::process::exit", "core::process::exit"],
        ))
    } else {
        None
    };

    let mut locals = Vec::with_capacity(body.locals.len());
    let mut local_indices = HashMap::new();
    for (idx, (local_id, local)) in body.locals.iter().enumerate() {
        let ty = if is_rust_entry_main && idx == 0 {
            Ty::new_tuple(&[])
        } else {
            local.ty
        };
        locals.push(MirLocalDecl {
            ty,
            span,
            mutability: Mutability::Mut,
        });
        local_indices.insert(local_id, idx);
    }

    let mut builder = Builder {
        deps,
        ctx,
        file_id,
        local_indices,
        locals,
        extra_locals: Vec::new(),
        blocks: Vec::new(),
        stmts: Vec::new(),
        span,
        is_rust_entry_main,
        exit_fn,
        exit_code_local: None,
    };

    if is_rust_entry_main {
        let i32_ty = Ty::signed_ty(IntTy::I32);
        let local = builder.new_temp(i32_ty, Mutability::Mut, span);
        let zero = MirConst::try_from_uint(0, UintTy::U32).expect("failed to build zero const");
        builder.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(local),
                Rvalue::Cast(
                    CastKind::IntToInt,
                    MirOperand::Constant(ConstOperand {
                        span,
                        user_ty: None,
                        const_: zero,
                    }),
                    i32_ty,
                ),
            ),
            span,
        });
        builder.exit_code_local = Some(local);
    }

    for stmt in &body.stmts {
        builder.lower_stmt(stmt);
    }

    builder.terminate_fallthrough();
    builder.locals.extend(builder.extra_locals);

    Body::new(
        builder.blocks,
        builder.locals,
        body.params.len(),
        vec![],
        None,
        span,
    )
}

struct Builder<'a, 'tcx> {
    deps: &'a rustc_gen::DependencyInfo,
    ctx: &'a rustc_gen::HirStructureCtx<'tcx>,
    file_id: rustc_gen::FileId,
    local_indices: HashMap<LocalId, usize>,
    locals: Vec<MirLocalDecl>,
    extra_locals: Vec<MirLocalDecl>,
    blocks: Vec<MirBasicBlock>,
    stmts: Vec<MirStatement>,
    span: RustSpan,
    is_rust_entry_main: bool,
    exit_fn: Option<FnDef>,
    exit_code_local: Option<usize>,
}

impl Builder<'_, '_> {
    fn lower_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Decl(HirDecl {
                local, initializer, ..
            }) => {
                if let Some(init) = initializer {
                    let local_index = self.local_to_index(*local);
                    if let HirExprKind::Aggregate { args } = &init.kind {
                        let TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) = init.ty.kind() else {
                            panic!("aggregate initializer expects adt type, got {:?}", init.ty);
                        };
                        let mut operands = Vec::with_capacity(args.len());
                        for arg in args {
                            operands.push(self.lower_expr_to_operand(arg));
                        }
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(local_index),
                                Rvalue::Aggregate(
                                    AggregateKind::Adt(adt, variant_idx(0), adt_args, None, None),
                                    operands,
                                ),
                            ),
                            span: self.hir_span(init.span),
                        });
                    } else if let HirExprKind::Call { func, args } = &init.kind {
                        let local_ty = self.locals[local_index].ty;
                        self.lower_call_to_destination(
                            func,
                            args,
                            init.span,
                            place(local_index),
                            local_ty,
                        );
                    } else {
                        let value = self.lower_expr_to_operand(init);
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(place(local_index), Rvalue::Use(value)),
                            span: self.hir_span(init.span),
                        });
                    }
                }
            }
            HirStmt::Expr(expr) => {
                let _ = self.lower_expr_to_operand(expr);
            }
            HirStmt::Return(expr, span) => {
                if let Some(expr) = expr {
                    if self.is_rust_entry_main {
                        let mut value = self.lower_expr_to_operand(expr);
                        if expr.ty != Ty::signed_ty(IntTy::I32) {
                            let cast_local = self.new_temp(
                                Ty::signed_ty(IntTy::I32),
                                Mutability::Mut,
                                self.hir_span(expr.span),
                            );
                            self.stmts.push(MirStatement {
                                kind: MirStatementKind::Assign(
                                    place(cast_local),
                                    Rvalue::Cast(
                                        CastKind::IntToInt,
                                        value,
                                        Ty::signed_ty(IntTy::I32),
                                    ),
                                ),
                                span: self.hir_span(expr.span),
                            });
                            value = MirOperand::Copy(place(cast_local));
                        }
                        self.stmts.push(MirStatement {
                            kind: MirStatementKind::Assign(
                                place(self.exit_code_local.expect("missing exit code local")),
                                Rvalue::Use(value),
                            ),
                            span: self.hir_span(expr.span),
                        });
                    } else {
                        if let HirExprKind::Call { func, args } = &expr.kind {
                            self.lower_call_to_destination(
                                func,
                                args,
                                expr.span,
                                place(0),
                                self.locals[0].ty,
                            );
                        } else {
                            let value = self.lower_expr_to_operand(expr);
                            self.stmts.push(MirStatement {
                                kind: MirStatementKind::Assign(place(0), Rvalue::Use(value)),
                                span: self.hir_span(expr.span),
                            });
                        }
                    }
                }

                if self.is_rust_entry_main {
                    self.push_exit_terminator(self.hir_span(*span));
                } else {
                    self.push_terminator(TerminatorKind::Return, self.hir_span(*span));
                }
            }
        }
    }

    fn terminate_fallthrough(&mut self) {
        if self.is_rust_entry_main {
            self.push_exit_terminator(self.span);
        } else {
            self.push_terminator(TerminatorKind::Return, self.span);
        }
    }

    fn lower_expr_to_operand(&mut self, expr: &HirExpr) -> MirOperand {
        match &expr.kind {
            HirExprKind::Local(local) => {
                let local_index = self.local_to_index(*local);
                match self.locals[local_index].ty.kind() {
                    TyKind::RigidTy(RigidTy::Adt(_, _)) => MirOperand::Move(place(local_index)),
                    _ => MirOperand::Copy(place(local_index)),
                }
            }
            HirExprKind::ConstInt(v) => {
                let span = self.hir_span(expr.span);
                let (uint_ty, bits) = int_literal_bits(*v, expr.ty);
                let c = MirConst::try_from_uint(bits, uint_ty).expect("failed to build int const");
                let const_op = MirOperand::Constant(ConstOperand {
                    span,
                    user_ty: None,
                    const_: c,
                });

                if matches!(expr.ty.kind(), TyKind::RigidTy(RigidTy::Uint(_))) {
                    return const_op;
                }

                let tmp = self.new_temp(expr.ty, Mutability::Mut, span);
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::Cast(CastKind::IntToInt, const_op, expr.ty),
                    ),
                    span,
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::Field { .. } => {
                let place = self
                    .lower_expr_to_place(expr)
                    .expect("field expression should be place-expressible");
                MirOperand::Move(place)
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr_to_operand(lhs);
                let rhs = self.lower_expr_to_operand(rhs);
                let tmp = self.new_temp(expr.ty, Mutability::Mut, self.hir_span(expr.span));
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::BinaryOp(self.lower_bin_op(*op), lhs, rhs),
                    ),
                    span: self.hir_span(expr.span),
                });
                MirOperand::Move(place(tmp))
            }
            HirExprKind::Aggregate { args } => {
                let TyKind::RigidTy(RigidTy::Adt(adt, adt_args)) = expr.ty.kind() else {
                    panic!("aggregate initializer expects adt type, got {:?}", expr.ty);
                };
                let mut operands = Vec::with_capacity(args.len());
                for arg in args {
                    operands.push(self.lower_expr_to_operand(arg));
                }
                let tmp = self.new_temp(expr.ty, Mutability::Mut, self.hir_span(expr.span));
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(tmp),
                        Rvalue::Aggregate(
                            AggregateKind::Adt(adt, variant_idx(0), adt_args, None, None),
                            operands,
                        ),
                    ),
                    span: self.hir_span(expr.span),
                });
                MirOperand::Copy(place(tmp))
            }
            HirExprKind::ConstStr(s) => self.lower_const_string(s, expr.span),
            HirExprKind::Path(path) => {
                if let ResolvedValue::Fn(fn_def) = path {
                    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(vec![])));
                    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
                    MirOperand::Constant(ConstOperand {
                        span: self.hir_span(expr.span),
                        user_ty: None,
                        const_: c,
                    })
                } else {
                    panic!("non-callable path in value position: {:?}", path);
                }
            }
            HirExprKind::Call { func, args } => {
                self.lower_call_expr(func, args, expr.span, expr.ty)
            }
        }
    }

    fn lower_expr_to_place(&mut self, expr: &HirExpr) -> Option<MirPlace> {
        match &expr.kind {
            HirExprKind::Local(local) => Some(place(self.local_to_index(*local))),
            HirExprKind::Field { base, index } => {
                let mut base_place = self.lower_expr_to_place(base)?;
                base_place
                    .projection
                    .push(MirProjection::Field(*index, expr.ty));
                Some(base_place)
            }
            _ => None,
        }
    }

    fn lower_bin_op(&self, op: HirBinOp) -> MirBinOp {
        match op {
            HirBinOp::Add => MirBinOp::Add,
            HirBinOp::Sub => MirBinOp::Sub,
            HirBinOp::Mul => MirBinOp::Mul,
        }
    }

    fn lower_call_expr(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: co2_hir::Span,
        ret_ty: Ty,
    ) -> MirOperand {
        let ret_local = self.new_temp(ret_ty, Mutability::Mut, self.hir_span(span));
        self.lower_call_to_destination(func, args, span, place(ret_local), ret_ty);
        MirOperand::Copy(place(ret_local))
    }

    fn lower_call_to_destination(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: co2_hir::Span,
        destination: MirPlace,
        ret_ty: Ty,
    ) {
        let fn_def = match &func.kind {
            HirExprKind::Path(ResolvedValue::Fn(fn_def)) => *fn_def,
            _ => panic!("unsupported call target: {:?}", func.kind),
        };

        let mut arg_ops = Vec::with_capacity(args.len());
        for arg in args {
            let op = if let HirExprKind::Local(local) = arg.kind {
                MirOperand::Move(place(self.local_to_index(local)))
            } else {
                self.lower_expr_to_operand(arg)
            };
            arg_ops.push(op);
        }

        let _ = ret_ty;
        self.emit_call_block(
            fn_const_operand(fn_def, vec![], self.hir_span(span)),
            arg_ops,
            destination,
            self.hir_span(span),
        );
    }

    #[cfg(false)]
    fn lower_call_to_destination(
        &mut self,
        func: &HirExpr,
        args: &[HirExpr],
        span: co2_hir::Span,
        destination: MirPlace,
        ret_ty: Ty,
    ) {
        let HirExprKind::Path(path) = &func.kind else {
            panic!("unsupported call target: {:?}", func.kind);
        };

        let fn_def = if let ResolvedValue::Fn(fn_def) = path {
            fn_def
        } else {
            todo!()
        };

        let mut arg_ops = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            if idx == 0
                && let Some(borrow_kind) = autoref_kind_for_path(&path.path)
                && let HirExprKind::Local(local) = arg.kind
            {
                let local = self.local_to_index(local);
                let region = Region {
                    kind: RegionKind::ReErased,
                };
                let ref_ty = Ty::new_ref(region.clone(), self.locals[local].ty, borrow_kind.1);
                let ref_local = self.new_temp(ref_ty, Mutability::Not, self.hir_span(arg.span));
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(ref_local),
                        Rvalue::Ref(region, borrow_kind.0, place(local)),
                    ),
                    span: self.hir_span(arg.span),
                });
                arg_ops.push(MirOperand::Move(place(ref_local)));
                continue;
            }

            let mut op = if let HirExprKind::Local(local) = arg.kind {
                MirOperand::Move(place(self.local_to_index(local)))
            } else {
                self.lower_expr_to_operand(arg)
            };
            if idx == 0 && (path.path == "printf" || path.path.ends_with("::printf")) {
                let const_ptr_i8 = Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Not);
                let cast_local =
                    self.new_temp(const_ptr_i8, Mutability::Mut, self.hir_span(arg.span));
                self.stmts.push(MirStatement {
                    kind: MirStatementKind::Assign(
                        place(cast_local),
                        Rvalue::Cast(CastKind::PtrToPtr, op, const_ptr_i8),
                    ),
                    span: self.hir_span(arg.span),
                });
                op = MirOperand::Copy(place(cast_local));
            }
            arg_ops.push(op);
        }

        let _ = ret_ty;
        if std::env::var_os("GEN_DEBUG").is_some() {
            eprintln!("co2_mir call {} arg_count={}", path.path, args.len());
        }
        self.emit_call_block(
            fn_const_operand(fn_def, vec![], self.hir_span(span)),
            arg_ops,
            destination,
            self.hir_span(span),
        );
    }

    fn lower_const_string(&mut self, s: &str, span: co2_hir::Span) -> MirOperand {
        let span = self.hir_span(span);
        let mut value = s.to_owned();
        if !value.ends_with('\0') {
            value.push('\0');
        }

        let as_ptr = dep_fn_any(self.deps, &["core::str::as_ptr", "std::str::as_ptr"]);
        let ptr_u8_ty = Ty::new_ptr(Ty::unsigned_ty(UintTy::U8), Mutability::Not);
        let ptr_u8_local = self.new_temp(ptr_u8_ty, Mutability::Mut, span);
        self.emit_call_block(
            fn_const_operand(as_ptr, vec![], span),
            vec![MirOperand::Constant(ConstOperand {
                span,
                user_ty: None,
                const_: MirConst::from_str(&value),
            })],
            place(ptr_u8_local),
            span,
        );

        let ptr_i8_ty = Ty::new_ptr(Ty::signed_ty(IntTy::I8), Mutability::Mut);
        let ptr_i8_local = self.new_temp(ptr_i8_ty, Mutability::Mut, span);
        self.stmts.push(MirStatement {
            kind: MirStatementKind::Assign(
                place(ptr_i8_local),
                Rvalue::Cast(
                    CastKind::PtrToPtr,
                    MirOperand::Copy(place(ptr_u8_local)),
                    ptr_i8_ty,
                ),
            ),
            span,
        });

        MirOperand::Copy(place(ptr_i8_local))
    }

    fn emit_call_block(
        &mut self,
        func: MirOperand,
        args: Vec<MirOperand>,
        destination: MirPlace,
        span: RustSpan,
    ) {
        let next = self.blocks.len() + 1;
        self.blocks.push(MirBasicBlock {
            statements: std::mem::take(&mut self.stmts),
            terminator: MirTerminator {
                kind: TerminatorKind::Call {
                    func,
                    args,
                    destination,
                    target: Some(next),
                    unwind: UnwindAction::Continue,
                },
                span,
            },
        });
    }

    fn push_exit_terminator(&mut self, span: RustSpan) {
        self.push_terminator(
            TerminatorKind::Call {
                func: fn_const_operand(self.exit_fn.expect("missing exit fn"), vec![], span),
                args: vec![MirOperand::Copy(place(
                    self.exit_code_local.expect("missing exit code local"),
                ))],
                destination: place(0),
                target: None,
                unwind: UnwindAction::Continue,
            },
            span,
        );
    }

    fn push_terminator(&mut self, kind: TerminatorKind, span: RustSpan) {
        self.blocks.push(MirBasicBlock {
            statements: std::mem::take(&mut self.stmts),
            terminator: MirTerminator { kind, span },
        });
    }

    fn new_temp(&mut self, ty: Ty, mutability: Mutability, span: RustSpan) -> usize {
        let local = self.locals.len() + self.extra_locals.len();
        self.extra_locals.push(MirLocalDecl {
            ty,
            span,
            mutability,
        });
        local
    }

    fn hir_span(&self, span: co2_hir::Span) -> RustSpan {
        self.ctx
            .span_in_file(self.file_id, span.start as u32, span.end as u32)
    }

    fn local_to_index(&self, local: LocalId) -> usize {
        *self
            .local_indices
            .get(&local)
            .unwrap_or_else(|| panic!("missing MIR local mapping for {local:?}"))
    }
}

fn int_literal_bits(value: i64, target_ty: Ty) -> (UintTy, u128) {
    let TyKind::RigidTy(rigid) = target_ty.kind() else {
        return (UintTy::U32, value as i32 as u32 as u128);
    };

    match rigid {
        RigidTy::Int(IntTy::I8) => (UintTy::U8, value as i8 as u8 as u128),
        RigidTy::Int(IntTy::I16) => (UintTy::U16, value as i16 as u16 as u128),
        RigidTy::Int(IntTy::I32) => (UintTy::U32, value as i32 as u32 as u128),
        RigidTy::Int(IntTy::I64) => (UintTy::U64, value as u64 as u128),
        RigidTy::Int(IntTy::I128) => (UintTy::U128, value as i128 as u128),
        RigidTy::Int(IntTy::Isize) => (UintTy::Usize, value as isize as usize as u128),
        RigidTy::Uint(UintTy::U8) => (UintTy::U8, value as u8 as u128),
        RigidTy::Uint(UintTy::U16) => (UintTy::U16, value as u16 as u128),
        RigidTy::Uint(UintTy::U32) => (UintTy::U32, value as u32 as u128),
        RigidTy::Uint(UintTy::U64) => (UintTy::U64, value as u64 as u128),
        RigidTy::Uint(UintTy::U128) => (UintTy::U128, value as u128),
        RigidTy::Uint(UintTy::Usize) => (UintTy::Usize, value as usize as u128),
        _ => (UintTy::U32, value as i32 as u32 as u128),
    }
}

fn place(local: usize) -> MirPlace {
    MirPlace {
        local,
        projection: vec![],
    }
}

fn variant_idx(id: usize) -> VariantIdx {
    unsafe { std::mem::transmute::<usize, VariantIdx>(id) }
}

fn generic_args_from_ty(ty: Ty) -> Vec<GenericArgKind> {
    match ty.kind() {
        TyKind::RigidTy(RigidTy::Adt(_, GenericArgs(args))) => args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgKind::Type(ty) => Some(GenericArgKind::Type(*ty)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn fn_const_operand(
    fn_def: FnDef,
    generic_args: Vec<GenericArgKind>,
    span: RustSpan,
) -> MirOperand {
    let fn_ty = Ty::from_rigid_kind(RigidTy::FnDef(fn_def, GenericArgs(generic_args)));
    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn dep_fn_any(deps: &rustc_gen::DependencyInfo, paths: &[&str]) -> FnDef {
    for path in paths {
        if let Some(found) = find_dep_fn(deps, path) {
            return found;
        }
    }
    panic!("missing dependency function (any of): {}", paths.join(", "));
}

fn dep_fn_for_path(deps: &rustc_gen::DependencyInfo, path: &str) -> FnDef {
    if path == "printf" || path.ends_with("::printf") {
        return dep_fn_any(deps, &["libc::printf", "libc::unix::printf"]);
    }
    if path.ends_with("::Option::unwrap") || path.ends_with("::option::Option::unwrap") {
        return dep_fn_any(
            deps,
            &[
                "std::option::Option::unwrap",
                "core::option::Option::unwrap",
            ],
        );
    }
    if path.ends_with("::Result::unwrap") || path.ends_with("::result::Result::unwrap") {
        return dep_fn_any(
            deps,
            &[
                "std::result::Result::unwrap",
                "core::result::Result::unwrap",
            ],
        );
    }
    if path.ends_with("::Iterator::nth") {
        return dep_fn_any(
            deps,
            &[
                "std::iter::Iterator::nth",
                "core::iter::traits::iterator::Iterator::nth",
            ],
        );
    }
    if path.ends_with("::Iterator::next") {
        return dep_fn_any(
            deps,
            &[
                "std::iter::Iterator::next",
                "core::iter::traits::iterator::Iterator::next",
            ],
        );
    }
    if path.ends_with("::Vec::as_mut_ptr") {
        return dep_fn_any(
            deps,
            &["std::vec::Vec::as_mut_ptr", "alloc::vec::Vec::as_mut_ptr"],
        );
    }
    if path.ends_with("::Vec::as_ptr") {
        return dep_fn_any(deps, &["std::vec::Vec::as_ptr", "alloc::vec::Vec::as_ptr"]);
    }
    if path.ends_with("::Vec::len") {
        return dep_fn_any(deps, &["std::vec::Vec::len", "alloc::vec::Vec::len"]);
    }
    dep_fn(deps, path)
}

fn dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> FnDef {
    if let Some(found) = find_dep_fn(deps, path) {
        return found;
    }
    panic!("missing dependency function: {path}");
}

fn find_dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> Option<FnDef> {
    let normalized_path = normalize_dep_path(path);

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| normalize_dep_path(&f.path) == normalized_path && f.fn_def.is_some())
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| {
            let normalized = normalize_dep_path(&f.path);
            (if path.contains("::") {
                normalized.ends_with(&normalized_path)
            } else {
                normalized.ends_with(&format!("::{}", normalized_path))
            }) && f.fn_def.is_some()
                && !f.path.contains("::{closure")
                && !f.path.contains("{{")
        })
        .and_then(|f| f.fn_def)
    {
        return Some(found);
    }

    if let Some(last) = path.rsplit("::").next() {
        let required_segments = normalized_path
            .split("::")
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                let normalized = normalize_dep_path(&f.path);
                normalized.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && !f.path.contains("::{closure")
                    && !f.path.contains("{{")
                    && required_segments.iter().all(|seg| normalized.contains(seg))
            })
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
        if let Some(found) = deps
            .functions
            .iter()
            .find(|f| {
                f.path.ends_with(&format!("::{last}"))
                    && f.fn_def.is_some()
                    && !f.path.contains("::{closure")
                    && !f.path.contains("{{")
            })
            .and_then(|f| f.fn_def)
        {
            return Some(found);
        }
    }

    None
}

fn normalize_dep_path(path: &str) -> String {
    let mut no_generics = String::with_capacity(path.len());
    let mut depth = 0usize;
    for ch in path.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => no_generics.push(ch),
            _ => {}
        }
    }
    no_generics
        .split("::")
        .filter(|seg| !seg.is_empty() && !seg.starts_with('{') && !seg.ends_with('}'))
        .collect::<Vec<_>>()
        .join("::")
}
