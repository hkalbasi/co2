#![feature(rustc_private)]

use rustc_public_generative as gen;

fn place(local: usize) -> gen::MirPlace {
    gen::MirPlace {
        local,
        projection: vec![],
    }
}

fn const_uint(value: u128, span: gen::PublicSpan) -> gen::MirOperand {
    let c = gen::PublicMirConst::try_from_uint(value, gen::PublicUintTy::Usize)
        .expect("failed to build usize const");
    gen::MirOperand::Constant(gen::MirConst {
        span,
        user_ty: None,
        const_: c,
    })
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

fn dep_fn(deps: &gen::DependencyInfo, path: &str) -> gen::FnDef {
    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| f.path == path && f.fn_def.is_some())
        .and_then(|f| f.fn_def)
    {
        return found;
    }

    if let Some(found) = deps
        .functions
        .iter()
        .find(|f| f.path.ends_with(path) && f.fn_def.is_some())
        .and_then(|f| f.fn_def)
    {
        return found;
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
            return found;
        }
    }

    let mut similar = deps
        .functions
        .iter()
        .filter(|f| {
            f.path.contains(path)
                || path.contains(&f.path)
                || path
                    .rsplit("::")
                    .next()
                    .is_some_and(|last| f.path.ends_with(&format!("::{last}")))
        })
        .map(|f| format!("{} (fn_def={})", f.path, f.fn_def.is_some()))
        .collect::<Vec<_>>();
    similar.sort();
    similar.truncate(20);
    panic!(
        "missing dependency function: {path}\nexample matches:\n{}",
        similar.join("\n")
    );
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
            .find(|t| t.path.ends_with(&format!("::{last}")) && !t.path.contains("{{"))
            .map(|t| t.adt)
        {
            return found;
        }
    }

    let mut similar = deps
        .types
        .iter()
        .filter(|t| {
            t.path.contains(path)
                || path.contains(&t.path)
                || path
                    .rsplit("::")
                    .next()
                    .is_some_and(|last| t.path.ends_with(&format!("::{last}")))
        })
        .map(|t| t.path.clone())
        .collect::<Vec<_>>();
    similar.sort();
    similar.truncate(20);
    panic!(
        "missing dependency type: {path}\nexample matches:\n{}",
        similar.join("\n")
    );
}

fn main() {
    let item_main = gen::ItemId::new(1);
    let item_write = gen::ItemId::new(2);
    let item_open = gen::ItemId::new(3);
    let item_read = gen::ItemId::new(4);
    let item_close = gen::ItemId::new(5);
    let item_malloc = gen::ItemId::new(6);
    let item_free = gen::ItemId::new(7);

    gen::generate(
        move |_ctx, _deps| {
            let usize_ty = gen::MirTy::usize_ty();
            let i8_ty = gen::MirTy::signed_ty(gen::PublicIntTy::I8);
            let ptr_i8_mut = gen::MirTy::new_ptr(i8_ty, gen::MirMutability::Mut);

            gen::CurrentCrateInfo {
                crate_name: "fake_hello_world".to_string(),
                entry: Some(item_main),
                items: vec![
                    gen::ItemInfo {
                        id: item_main,
                        name: "main".to_string(),
                        parent: None,
                        kind: gen::ItemKind::Function,
                    },
                    gen::ItemInfo {
                        id: item_write,
                        name: "write".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![usize_ty, ptr_i8_mut, usize_ty],
                            output: usize_ty,
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                    gen::ItemInfo {
                        id: item_open,
                        name: "open".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![ptr_i8_mut, usize_ty],
                            output: usize_ty,
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                    gen::ItemInfo {
                        id: item_read,
                        name: "read".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![usize_ty, ptr_i8_mut, usize_ty],
                            output: usize_ty,
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                    gen::ItemInfo {
                        id: item_close,
                        name: "close".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![usize_ty],
                            output: usize_ty,
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                    gen::ItemInfo {
                        id: item_malloc,
                        name: "malloc".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![usize_ty],
                            output: ptr_i8_mut,
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                    gen::ItemInfo {
                        id: item_free,
                        name: "free".to_string(),
                        parent: None,
                        kind: gen::ItemKind::ForeignFunction(gen::FunctionSignature {
                            inputs: vec![ptr_i8_mut],
                            output: gen::MirTy::new_tuple(&[]),
                            abi: gen::FunctionAbi::C,
                            is_unsafe: true,
                        }),
                    },
                ],
            }
        },
        move |_ctx, deps, defined| {
            let span: gen::PublicSpan = unsafe { std::mem::zeroed() };

            let write = defined
                .items
                .iter()
                .find(|i| i.id == item_write)
                .and_then(|i| i.fn_def())
                .expect("missing write def");
            let open = defined
                .items
                .iter()
                .find(|i| i.id == item_open)
                .and_then(|i| i.fn_def())
                .expect("missing open def");
            let read = defined
                .items
                .iter()
                .find(|i| i.id == item_read)
                .and_then(|i| i.fn_def())
                .expect("missing read def");
            let close = defined
                .items
                .iter()
                .find(|i| i.id == item_close)
                .and_then(|i| i.fn_def())
                .expect("missing close def");
            let malloc = defined
                .items
                .iter()
                .find(|i| i.id == item_malloc)
                .and_then(|i| i.fn_def())
                .expect("missing malloc def");
            let free = defined
                .items
                .iter()
                .find(|i| i.id == item_free)
                .and_then(|i| i.fn_def())
                .expect("missing free def");

            let std_env_args = dep_fn(&deps, "std::env::args");
            let iter_nth = dep_fn(&deps, "std::iter::Iterator::nth");
            let option_unwrap = dep_fn(&deps, "std::option::Option::unwrap");
            let result_unwrap = dep_fn(&deps, "std::result::Result::unwrap");
            let cstring_new = dep_fn(&deps, "std::ffi::CString::new");
            let cstring_into_raw = dep_fn(&deps, "std::ffi::CString::into_raw");

            let args_adt = dep_adt(&deps, "std::env::Args");
            let string_adt = dep_adt(&deps, "std::string::String");
            let cstring_adt = dep_adt(&deps, "std::ffi::CString");
            let nul_error_adt = dep_adt(&deps, "std::ffi::NulError");
            let option_adt = dep_adt(&deps, "std::option::Option");
            let result_adt = dep_adt(&deps, "std::result::Result");

            let args_ty =
                gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(args_adt, gen::GenericArgs(vec![])));
            let string_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
                string_adt,
                gen::GenericArgs(vec![]),
            ));
            let cstring_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
                cstring_adt,
                gen::GenericArgs(vec![]),
            ));
            let nul_error_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
                nul_error_adt,
                gen::GenericArgs(vec![]),
            ));
            let option_string_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
                option_adt,
                gen::GenericArgs(vec![gen::GenericArgKind::Type(string_ty)]),
            ));
            let result_cstring_ty = gen::MirTy::from_rigid_kind(gen::RigidTy::Adt(
                result_adt,
                gen::GenericArgs(vec![
                    gen::GenericArgKind::Type(cstring_ty),
                    gen::GenericArgKind::Type(nul_error_ty),
                ]),
            ));

            let args_ref_ty = gen::MirTy::new_ref(
                gen::Region {
                    kind: gen::RegionKind::ReErased,
                },
                args_ty,
                gen::MirMutability::Mut,
            );
            let usize_ty = gen::MirTy::usize_ty();
            let i8_ty = gen::MirTy::signed_ty(gen::PublicIntTy::I8);
            let ptr_i8_mut = gen::MirTy::new_ptr(i8_ty, gen::MirMutability::Mut);

            let locals = vec![
                gen::MirLocalDecl {
                    ty: gen::MirTy::new_tuple(&[]),
                    span,
                    mutability: gen::MirMutability::Not,
                },
                gen::MirLocalDecl {
                    ty: args_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: args_ref_ty,
                    span,
                    mutability: gen::MirMutability::Not,
                },
                gen::MirLocalDecl {
                    ty: option_string_ty,
                    span,
                    mutability: gen::MirMutability::Not,
                },
                gen::MirLocalDecl {
                    ty: string_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: result_cstring_ty,
                    span,
                    mutability: gen::MirMutability::Not,
                },
                gen::MirLocalDecl {
                    ty: cstring_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: ptr_i8_mut,
                    span,
                    mutability: gen::MirMutability::Not,
                },
                gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: ptr_i8_mut,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
                gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: gen::MirMutability::Mut,
                },
            ];

            let blocks = vec![
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(std_env_args, vec![], span),
                            args: vec![],
                            destination: place(1),
                            target: Some(1),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![gen::MirStatement {
                        kind: gen::MirStatementKind::Assign(
                            place(2),
                            gen::MirRvalue::Ref(
                                gen::Region {
                                    kind: gen::RegionKind::ReErased,
                                },
                                gen::MirBorrowKind::Mut {
                                    kind: gen::MirMutBorrowKind::Default,
                                },
                                place(1),
                            ),
                        ),
                        span,
                    }],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                iter_nth,
                                vec![gen::GenericArgKind::Type(args_ty)],
                                span,
                            ),
                            args: vec![gen::MirOperand::Move(place(2)), const_uint(1, span)],
                            destination: place(3),
                            target: Some(2),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                option_unwrap,
                                vec![gen::GenericArgKind::Type(string_ty)],
                                span,
                            ),
                            args: vec![gen::MirOperand::Move(place(3))],
                            destination: place(4),
                            target: Some(3),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                cstring_new,
                                vec![gen::GenericArgKind::Type(string_ty)],
                                span,
                            ),
                            args: vec![gen::MirOperand::Move(place(4))],
                            destination: place(5),
                            target: Some(4),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                result_unwrap,
                                vec![
                                    gen::GenericArgKind::Type(cstring_ty),
                                    gen::GenericArgKind::Type(nul_error_ty),
                                ],
                                span,
                            ),
                            args: vec![gen::MirOperand::Move(place(5))],
                            destination: place(6),
                            target: Some(5),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(cstring_into_raw, vec![], span),
                            args: vec![gen::MirOperand::Move(place(6))],
                            destination: place(7),
                            target: Some(6),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(open, vec![], span),
                            args: vec![gen::MirOperand::Copy(place(7)), const_uint(0, span)],
                            destination: place(8),
                            target: Some(7),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(malloc, vec![], span),
                            args: vec![const_uint(4096, span)],
                            destination: place(9),
                            target: Some(8),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(read, vec![], span),
                            args: vec![
                                gen::MirOperand::Copy(place(8)),
                                gen::MirOperand::Copy(place(9)),
                                const_uint(4096, span),
                            ],
                            destination: place(10),
                            target: Some(9),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(write, vec![], span),
                            args: vec![
                                const_uint(1, span),
                                gen::MirOperand::Copy(place(9)),
                                gen::MirOperand::Copy(place(10)),
                            ],
                            destination: place(11),
                            target: Some(10),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(close, vec![], span),
                            args: vec![gen::MirOperand::Copy(place(8))],
                            destination: place(11),
                            target: Some(11),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Call {
                            func: fn_const_operand(free, vec![], span),
                            args: vec![gen::MirOperand::Copy(place(9))],
                            destination: place(0),
                            target: Some(12),
                            unwind: gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator {
                        kind: gen::MirTerminatorKind::Return,
                        span,
                    },
                },
            ];

            let body = gen::MirBody::new(blocks, locals, 0, vec![], None, span);
            vec![gen::ItemMirInfo {
                id: item_main,
                body,
            }]
        },
    );
}
