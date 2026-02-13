#![feature(rustc_private)]

use std::sync::OnceLock;

use rustc_public_generative::{self as rustc_gen, DefinedCrateInfo, FileId};

fn place(local: usize) -> rustc_gen::MirPlace {
    rustc_gen::MirPlace {
        local,
        projection: vec![],
    }
}

fn const_uint(value: u128, span: rustc_gen::PublicSpan) -> rustc_gen::MirOperand {
    let c = rustc_gen::PublicMirConst::try_from_uint(value, rustc_gen::PublicUintTy::Usize)
        .expect("failed to build usize const");
    rustc_gen::MirOperand::Constant(rustc_gen::MirConst {
        span,
        user_ty: None,
        const_: c,
    })
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

fn dep_fn(deps: &rustc_gen::DependencyInfo, path: &str) -> rustc_gen::FnDef {
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

fn dep_adt(deps: &rustc_gen::DependencyInfo, path: &str) -> rustc_gen::AdtDef {
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

static FILE_ID: OnceLock<FileId> = OnceLock::new();

fn lookup_id(krate: &DefinedCrateInfo, name: &str) -> rustc_gen::DefId {
    krate
        .items
        .iter()
        .find(|item| item.name == name)
        .unwrap()
        .def_id()
}

fn main() {
    // let item_main = rustc_gen::ItemId::new(1);
    // let item_write = rustc_gen::ItemId::new(2);
    // let item_open = rustc_gen::ItemId::new(3);
    // let item_read = rustc_gen::ItemId::new(4);
    // let item_close = rustc_gen::ItemId::new(5);
    // let item_malloc = rustc_gen::ItemId::new(6);
    // let item_free = rustc_gen::ItemId::new(7);

    rustc_gen::generate(
        move |ctx, _deps| {
            let file_id = ctx.add_custom_file("/tmp/echo_file.rs", "fn main()");
            _ = FILE_ID.set(file_id);

            rustc_gen::CurrentCrateInfo {
                crate_name: "fake_hello_world".to_string(),
                no_main: false,
                items: vec![
                    rustc_gen::ItemInfo {
                        name: "main".to_string(),
                        kind: rustc_gen::ItemKind::Function,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "write".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "open".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "read".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "close".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "malloc".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "free".to_string(),
                        kind: rustc_gen::ItemKind::ForeignFunction,
                        no_mangle: false,
                    },
                ],
            }
        },
        move |ctx, _deps, _defined| {
            let file_id = *FILE_ID.get().unwrap();
            let span = ctx.span_in_file(file_id, 2, 5);

            let item_main = lookup_id(&_defined, "main");
            let item_write = lookup_id(&_defined, "write");
            let item_open = lookup_id(&_defined, "open");
            let item_read = lookup_id(&_defined, "read");
            let item_close = lookup_id(&_defined, "close");
            let item_malloc = lookup_id(&_defined, "malloc");
            let item_free = lookup_id(&_defined, "free");

            let usize_ty = || rustc_gen::HirTy::usize_ty(span);
            let i8_ty = || rustc_gen::HirTy::signed_ty(rustc_gen::PublicIntTy::I8, span);
            let ptr_i8_mut = || rustc_gen::HirTy::new_ptr(i8_ty(), rustc_gen::MirMutability::Mut, span);
            vec![
                rustc_gen::ItemSignatureInfo {
                    id: item_main,
                    kind: rustc_gen::ItemSignatureKind::Function {
                        sig: rustc_gen::FunctionSignature {
                            inputs: vec![],
                            output: rustc_gen::HirTy::new_tuple(vec![], span),
                            abi: rustc_gen::FunctionAbi::Rust,
                            is_unsafe: false,
                        },
                        no_mangle: false,
                    },
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_write,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![usize_ty(), ptr_i8_mut(), usize_ty()],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_open,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![ptr_i8_mut(), usize_ty()],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_read,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![usize_ty(), ptr_i8_mut(), usize_ty()],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_close,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![usize_ty()],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_malloc,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![usize_ty()],
                            output: ptr_i8_mut(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: item_free,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![ptr_i8_mut()],
                            output: rustc_gen::HirTy::new_tuple(vec![], span),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
            ]
        },
        move |ctx, deps, defined| {
            let span: rustc_gen::PublicSpan = ctx.span_in_file(*FILE_ID.get().unwrap(), 0, 2);

            let write = defined
                .items
                .iter()
                .find(|i| i.name == "write")
                .and_then(|i| i.fn_def())
                .expect("missing write def");
            let open = defined
                .items
                .iter()
                .find(|i| i.name == "open")
                .and_then(|i| i.fn_def())
                .expect("missing open def");
            let read = defined
                .items
                .iter()
                .find(|i| i.name == "read")
                .and_then(|i| i.fn_def())
                .expect("missing read def");
            let close = defined
                .items
                .iter()
                .find(|i| i.name == "close")
                .and_then(|i| i.fn_def())
                .expect("missing close def");
            let malloc = defined
                .items
                .iter()
                .find(|i| i.name == "malloc")
                .and_then(|i| i.fn_def())
                .expect("missing malloc def");
            let free = defined
                .items
                .iter()
                .find(|i| i.name == "free")
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

            let args_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                args_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let string_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                string_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let cstring_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                cstring_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let nul_error_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                nul_error_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let option_string_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                option_adt,
                rustc_gen::GenericArgs(vec![rustc_gen::GenericArgKind::Type(string_ty)]),
            ));
            let result_cstring_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                result_adt,
                rustc_gen::GenericArgs(vec![
                    rustc_gen::GenericArgKind::Type(cstring_ty),
                    rustc_gen::GenericArgKind::Type(nul_error_ty),
                ]),
            ));

            let args_ref_ty = rustc_gen::MirTy::new_ref(
                rustc_gen::Region {
                    kind: rustc_gen::RegionKind::ReErased,
                },
                args_ty,
                rustc_gen::MirMutability::Mut,
            );
            let usize_ty = rustc_gen::MirTy::usize_ty();
            let i8_ty = rustc_gen::MirTy::signed_ty(rustc_gen::PublicIntTy::I8);
            let ptr_i8_mut = rustc_gen::MirTy::new_ptr(i8_ty, rustc_gen::MirMutability::Mut);

            let locals = vec![
                rustc_gen::MirLocalDecl {
                    ty: rustc_gen::MirTy::new_tuple(&[]),
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: args_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: args_ref_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Not,
                },
                rustc_gen::MirLocalDecl {
                    ty: option_string_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Not,
                },
                rustc_gen::MirLocalDecl {
                    ty: string_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: result_cstring_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Not,
                },
                rustc_gen::MirLocalDecl {
                    ty: cstring_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: ptr_i8_mut,
                    span,
                    mutability: rustc_gen::MirMutability::Not,
                },
                rustc_gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: ptr_i8_mut,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
                rustc_gen::MirLocalDecl {
                    ty: usize_ty,
                    span,
                    mutability: rustc_gen::MirMutability::Mut,
                },
            ];

            let blocks = vec![
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(std_env_args, vec![], span),
                            args: vec![],
                            destination: place(1),
                            target: Some(1),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![rustc_gen::MirStatement {
                        kind: rustc_gen::MirStatementKind::Assign(
                            place(2),
                            rustc_gen::MirRvalue::Ref(
                                rustc_gen::Region {
                                    kind: rustc_gen::RegionKind::ReErased,
                                },
                                rustc_gen::MirBorrowKind::Mut {
                                    kind: rustc_gen::MirMutBorrowKind::Default,
                                },
                                place(1),
                            ),
                        ),
                        span,
                    }],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                iter_nth,
                                vec![rustc_gen::GenericArgKind::Type(args_ty)],
                                span,
                            ),
                            args: vec![rustc_gen::MirOperand::Move(place(2)), const_uint(1, span)],
                            destination: place(3),
                            target: Some(2),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                option_unwrap,
                                vec![rustc_gen::GenericArgKind::Type(string_ty)],
                                span,
                            ),
                            args: vec![rustc_gen::MirOperand::Move(place(3))],
                            destination: place(4),
                            target: Some(3),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                cstring_new,
                                vec![rustc_gen::GenericArgKind::Type(string_ty)],
                                span,
                            ),
                            args: vec![rustc_gen::MirOperand::Move(place(4))],
                            destination: place(5),
                            target: Some(4),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(
                                result_unwrap,
                                vec![
                                    rustc_gen::GenericArgKind::Type(cstring_ty),
                                    rustc_gen::GenericArgKind::Type(nul_error_ty),
                                ],
                                span,
                            ),
                            args: vec![rustc_gen::MirOperand::Move(place(5))],
                            destination: place(6),
                            target: Some(5),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(cstring_into_raw, vec![], span),
                            args: vec![rustc_gen::MirOperand::Move(place(6))],
                            destination: place(7),
                            target: Some(6),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(open, vec![], span),
                            args: vec![rustc_gen::MirOperand::Copy(place(7)), const_uint(0, span)],
                            destination: place(8),
                            target: Some(7),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(malloc, vec![], span),
                            args: vec![const_uint(4096, span)],
                            destination: place(9),
                            target: Some(8),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(read, vec![], span),
                            args: vec![
                                rustc_gen::MirOperand::Copy(place(8)),
                                rustc_gen::MirOperand::Copy(place(9)),
                                const_uint(4096, span),
                            ],
                            destination: place(10),
                            target: Some(9),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(write, vec![], span),
                            args: vec![
                                const_uint(1, span),
                                rustc_gen::MirOperand::Copy(place(9)),
                                rustc_gen::MirOperand::Copy(place(10)),
                            ],
                            destination: place(11),
                            target: Some(10),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(close, vec![], span),
                            args: vec![rustc_gen::MirOperand::Copy(place(8))],
                            destination: place(11),
                            target: Some(11),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Call {
                            func: fn_const_operand(free, vec![], span),
                            args: vec![rustc_gen::MirOperand::Copy(place(9))],
                            destination: place(0),
                            target: Some(12),
                            unwind: rustc_gen::MirUnwindAction::Continue,
                        },
                        span,
                    },
                },
                rustc_gen::MirBasicBlock {
                    statements: vec![],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Return,
                        span,
                    },
                },
            ];

            let body = rustc_gen::MirBody::new(blocks, locals, 0, vec![], None, span);
            vec![rustc_gen::ItemMirInfo {
                id: lookup_id(&defined, "main"),
                body,
            }]
        },
    );
}
