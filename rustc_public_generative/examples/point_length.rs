#![feature(rustc_private)]

use std::sync::OnceLock;

use rustc_public_generative as rustc_gen;

fn place(local: usize) -> rustc_gen::MirPlace {
    rustc_gen::MirPlace {
        local,
        projection: vec![],
    }
}

fn place_fields(local: usize, fields: &[(usize, rustc_gen::MirTy)]) -> rustc_gen::MirPlace {
    rustc_gen::MirPlace {
        local,
        projection: fields
            .iter()
            .map(|(field, ty)| rustc_gen::MirProjection::Field(*field, *ty))
            .collect(),
    }
}

fn const_uint(
    value: u128,
    ty: rustc_gen::PublicUintTy,
    span: rustc_gen::PublicSpan,
) -> rustc_gen::MirOperand {
    let c =
        rustc_gen::PublicMirConst::try_from_uint(value, ty).expect("failed to build uint const");
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

fn variant_idx(value: usize) -> rustc_gen::VariantIdx {
    unsafe { std::mem::transmute::<usize, rustc_gen::VariantIdx>(value) }
}

static FILE_ID: OnceLock<rustc_gen::FileId> = OnceLock::new();

fn main() {
    rustc_gen::generate(
        move |ctx, _deps| {
            let file_id = ctx.add_custom_file("/tmp/point_length.rs", "fn main()");
            _ = FILE_ID.set(file_id);
            rustc_gen::CurrentCrateInfo {
                crate_name: "point_length_fake".to_string(),
                no_main: false,
                items: vec![
                    rustc_gen::ItemInfo {
                        name: "Point".to_string(),
                        kind: rustc_gen::ItemKind::Struct(vec!["x".to_string(), "y".to_string()]),
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "Human".to_string(),
                        kind: rustc_gen::ItemKind::Struct(vec![
                            "location".to_string(),
                            "age".to_string(),
                        ]),
                        no_mangle: false,
                    },
                    rustc_gen::ItemInfo {
                        name: "length".to_string(),
                        kind: rustc_gen::ItemKind::Function,
                        no_mangle: false,
                    },
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
                ],
            }
        },
        move |ctx, _deps, defined| {
            let file_id = *FILE_ID.get().unwrap();
            let span = ctx.span_in_file(file_id, 2, 5);

            let point_adt = defined
                .items
                .iter()
                .find(|i| i.name == "Point")
                .and_then(|i| i.adt_def())
                .expect("missing point adt");
            let human_adt = defined
                .items
                .iter()
                .find(|i| i.name == "Human")
                .and_then(|i| i.adt_def())
                .expect("missing human adt");
            let usize_ty = || rustc_gen::HirTy::usize_ty(span);
            let u32_ty = rustc_gen::HirTy::unsigned_ty(rustc_gen::PublicUintTy::U32, span);
            let u8_ty = rustc_gen::HirTy::unsigned_ty(rustc_gen::PublicUintTy::U8, span);
            let ptr_u8_mut = rustc_gen::HirTy::new_ptr(u8_ty, rustc_gen::MirMutability::Mut, span);
            let point_ty = rustc_gen::HirTy::adt(point_adt, vec![], span);
            let human_ty = rustc_gen::HirTy::adt(human_adt, vec![], span);

            let length_fn = defined
                .items
                .iter()
                .find(|i| i.name == "length")
                .and_then(|i| i.fn_def())
                .expect("missing length");

            let main_fn = defined
                .items
                .iter()
                .find(|i| i.name == "main")
                .and_then(|i| i.fn_def())
                .expect("missing main");
            let write_fn = defined
                .items
                .iter()
                .find(|i| i.name == "write")
                .and_then(|i| i.fn_def())
                .expect("missing write");

            let x_field = defined
                .items
                .iter()
                .find(|i| i.name == "x")
                .map(|i| i.def_id())
                .expect("missing x");
            let y_field = defined
                .items
                .iter()
                .find(|i| i.name == "y")
                .map(|i| i.def_id())
                .expect("missing y");
            let location_field = defined
                .items
                .iter()
                .find(|i| i.name == "location")
                .map(|i| i.def_id())
                .expect("missing location");
            let age_field = defined
                .items
                .iter()
                .find(|i| i.name == "age")
                .map(|i| i.def_id())
                .expect("missing age");

            vec![
                rustc_gen::ItemSignatureInfo {
                    id: point_adt.0,
                    kind: rustc_gen::ItemSignatureKind::Struct(vec![
                        rustc_gen::StructField {
                            id: x_field,
                            ty: usize_ty(),
                        },
                        rustc_gen::StructField {
                            id: y_field,
                            ty: usize_ty(),
                        },
                    ]),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: human_adt.0,
                    kind: rustc_gen::ItemSignatureKind::Struct(vec![
                        rustc_gen::StructField {
                            id: location_field,
                            ty: point_ty,
                        },
                        rustc_gen::StructField {
                            id: age_field,
                            ty: u32_ty,
                        },
                    ]),
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: length_fn.0,
                    kind: rustc_gen::ItemSignatureKind::Function {
                        sig: rustc_gen::FunctionSignature {
                            inputs: vec![human_ty],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::Rust,
                            is_unsafe: false,
                        },
                        no_mangle: false,
                    },
                    span,
                },
                rustc_gen::ItemSignatureInfo {
                    id: main_fn.0,
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
                    id: write_fn.0,
                    kind: rustc_gen::ItemSignatureKind::ForeignFunction(
                        rustc_gen::FunctionSignature {
                            inputs: vec![usize_ty(), ptr_u8_mut, usize_ty()],
                            output: usize_ty(),
                            abi: rustc_gen::FunctionAbi::C,
                            is_unsafe: true,
                        },
                    ),
                    span,
                },
            ]
        },
        move |ctx, _deps, defined| {
            let file_id = *FILE_ID.get().unwrap();
            let span = ctx.span_in_file(file_id, 2, 5);

            let span: rustc_gen::PublicSpan = ctx.span_in_file(*FILE_ID.get().unwrap(), 0, 2);
            let point_adt = defined
                .items
                .iter()
                .find(|i| i.name == "Point")
                .and_then(|i| i.adt_def())
                .expect("missing point adt");
            let human_adt = defined
                .items
                .iter()
                .find(|i| i.name == "Human")
                .and_then(|i| i.adt_def())
                .expect("missing human adt");
            let usize_ty = rustc_gen::MirTy::usize_ty();
            let u8_ty = rustc_gen::MirTy::unsigned_ty(rustc_gen::PublicUintTy::U8);
            let point_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                point_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let human_ty = rustc_gen::MirTy::from_rigid_kind(rustc_gen::RigidTy::Adt(
                human_adt,
                rustc_gen::GenericArgs(vec![]),
            ));
            let tuple_u8_3_ty = rustc_gen::MirTy::new_tuple(&[u8_ty, u8_ty, u8_ty]);
            let ptr_tuple_u8_3 =
                rustc_gen::MirTy::new_ptr(tuple_u8_3_ty, rustc_gen::MirMutability::Mut);
            let ptr_u8_mut = rustc_gen::MirTy::new_ptr(u8_ty, rustc_gen::MirMutability::Mut);
            let length_fn = defined
                .items
                .iter()
                .find(|i| i.name == "length")
                .and_then(|i| i.fn_def())
                .expect("missing length fn def");
            // dbg!(length_fn.fn_sig());
            let write_fn = defined
                .items
                .iter()
                .find(|i| i.name == "write")
                .and_then(|i| i.fn_def())
                .expect("missing write fn def");
            let main_fn = defined
                .items
                .iter()
                .find(|i| i.name == "main")
                .and_then(|i| i.fn_def())
                .expect("missing write fn def");

            let length_body = {
                let locals = vec![
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: human_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Not,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Not,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Not,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Not,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Not,
                    },
                ];
                let blocks = vec![rustc_gen::MirBasicBlock {
                    statements: vec![
                        rustc_gen::MirStatement {
                            kind: rustc_gen::MirStatementKind::Assign(
                                place(2),
                                rustc_gen::MirRvalue::Use(rustc_gen::MirOperand::Move(
                                    place_fields(1, &[(0, point_ty), (0, usize_ty)]),
                                )),
                            ),
                            span,
                        },
                        rustc_gen::MirStatement {
                            kind: rustc_gen::MirStatementKind::Assign(
                                place(3),
                                rustc_gen::MirRvalue::Use(rustc_gen::MirOperand::Move(
                                    place_fields(1, &[(0, point_ty), (1, usize_ty)]),
                                )),
                            ),
                            span,
                        },
                        rustc_gen::MirStatement {
                            kind: rustc_gen::MirStatementKind::Assign(
                                place(4),
                                rustc_gen::MirRvalue::BinaryOp(
                                    rustc_gen::MirBinOp::Mul,
                                    rustc_gen::MirOperand::Move(place(2)),
                                    rustc_gen::MirOperand::Move(place(2)),
                                ),
                            ),
                            span,
                        },
                        rustc_gen::MirStatement {
                            kind: rustc_gen::MirStatementKind::Assign(
                                place(5),
                                rustc_gen::MirRvalue::BinaryOp(
                                    rustc_gen::MirBinOp::Mul,
                                    rustc_gen::MirOperand::Move(place(3)),
                                    rustc_gen::MirOperand::Move(place(3)),
                                ),
                            ),
                            span,
                        },
                        rustc_gen::MirStatement {
                            kind: rustc_gen::MirStatementKind::Assign(
                                place(0),
                                rustc_gen::MirRvalue::BinaryOp(
                                    rustc_gen::MirBinOp::Add,
                                    rustc_gen::MirOperand::Move(place(4)),
                                    rustc_gen::MirOperand::Move(place(5)),
                                ),
                            ),
                            span,
                        },
                    ],
                    terminator: rustc_gen::MirTerminator {
                        kind: rustc_gen::MirTerminatorKind::Return,
                        span,
                    },
                }];
                rustc_gen::MirBody::new(blocks, locals, 1, vec![], None, span)
            };

            let main_body = {
                let locals = vec![
                    rustc_gen::MirLocalDecl {
                        ty: rustc_gen::MirTy::new_tuple(&[]),
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: point_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: human_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: usize_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: tuple_u8_3_ty,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: ptr_tuple_u8_3,
                        span,
                        mutability: rustc_gen::MirMutability::Mut,
                    },
                    rustc_gen::MirLocalDecl {
                        ty: ptr_u8_mut,
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
                        statements: vec![
                            rustc_gen::MirStatement {
                                kind: rustc_gen::MirStatementKind::Assign(
                                    place(1),
                                    rustc_gen::MirRvalue::Aggregate(
                                        rustc_gen::MirAggregateKind::Adt(
                                            point_adt,
                                            variant_idx(0),
                                            rustc_gen::GenericArgs(vec![]),
                                            None,
                                            None,
                                        ),
                                        vec![
                                            const_uint(3, rustc_gen::PublicUintTy::Usize, span),
                                            const_uint(4, rustc_gen::PublicUintTy::Usize, span),
                                        ],
                                    ),
                                ),
                                span,
                            },
                            rustc_gen::MirStatement {
                                kind: rustc_gen::MirStatementKind::Assign(
                                    place(2),
                                    rustc_gen::MirRvalue::Aggregate(
                                        rustc_gen::MirAggregateKind::Adt(
                                            human_adt,
                                            variant_idx(0),
                                            rustc_gen::GenericArgs(vec![]),
                                            None,
                                            None,
                                        ),
                                        vec![
                                            rustc_gen::MirOperand::Move(place(1)),
                                            const_uint(30, rustc_gen::PublicUintTy::U32, span),
                                        ],
                                    ),
                                ),
                                span,
                            },
                        ],
                        terminator: rustc_gen::MirTerminator {
                            kind: rustc_gen::MirTerminatorKind::Call {
                                func: fn_const_operand(length_fn, vec![], span),
                                args: vec![rustc_gen::MirOperand::Move(place(2))],
                                destination: place(3),
                                target: Some(1),
                                unwind: rustc_gen::MirUnwindAction::Continue,
                            },
                            span,
                        },
                    },
                    rustc_gen::MirBasicBlock {
                        statements: vec![
                            rustc_gen::MirStatement {
                                kind: rustc_gen::MirStatementKind::Assign(
                                    place(4),
                                    rustc_gen::MirRvalue::Aggregate(
                                        rustc_gen::MirAggregateKind::Tuple,
                                        vec![
                                            const_uint(50, rustc_gen::PublicUintTy::U8, span),
                                            const_uint(53, rustc_gen::PublicUintTy::U8, span),
                                            const_uint(10, rustc_gen::PublicUintTy::U8, span),
                                        ],
                                    ),
                                ),
                                span,
                            },
                            rustc_gen::MirStatement {
                                kind: rustc_gen::MirStatementKind::Assign(
                                    place(5),
                                    rustc_gen::MirRvalue::AddressOf(
                                        rustc_gen::MirRawPtrKind::Mut,
                                        place(4),
                                    ),
                                ),
                                span,
                            },
                            rustc_gen::MirStatement {
                                kind: rustc_gen::MirStatementKind::Assign(
                                    place(6),
                                    rustc_gen::MirRvalue::Cast(
                                        rustc_gen::MirCastKind::PtrToPtr,
                                        rustc_gen::MirOperand::Move(place(5)),
                                        ptr_u8_mut,
                                    ),
                                ),
                                span,
                            },
                        ],
                        terminator: rustc_gen::MirTerminator {
                            kind: rustc_gen::MirTerminatorKind::Call {
                                func: fn_const_operand(write_fn, vec![], span),
                                args: vec![
                                    const_uint(1, rustc_gen::PublicUintTy::Usize, span),
                                    rustc_gen::MirOperand::Move(place(6)),
                                    const_uint(3, rustc_gen::PublicUintTy::Usize, span),
                                ],
                                destination: place(7),
                                target: Some(2),
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
                rustc_gen::MirBody::new(blocks, locals, 0, vec![], None, span)
            };

            vec![
                rustc_gen::ItemMirInfo {
                    id: length_fn.0,
                    body: length_body,
                },
                rustc_gen::ItemMirInfo {
                    id: main_fn.0,
                    body: main_body,
                },
            ]
        },
    );
}
