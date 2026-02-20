#![feature(rustc_private)]

use rustc_public_generative::{
    CrateGeneratorState, FileId, ForeignModItem, FunctionAbi, FunctionSignature,
    HirAdtKind, HirModule, HirModuleItem, HirStructure, HirStructureCtx, HirTy, StructField,
    generate,
    rustc_public::{
        DefId,
        mir::{
            BasicBlock as MirBasicBlock, Body, CastKind, ConstOperand, LocalDecl as MirLocalDecl,
            Mutability, Operand as MirOperand, Place as MirPlace, ProjectionElem as MirProjection,
            RawPtrKind, Rvalue, Statement as MirStatement, StatementKind as MirStatementKind,
            Terminator as MirTerminator, TerminatorKind, UnwindAction,
        },
        ty::{AdtDef, FnDef, GenericArgKind, GenericArgs, MirConst, Span, Ty, UintTy, VariantIdx},
    },
};

fn place(local: usize) -> MirPlace {
    MirPlace {
        local,
        projection: vec![],
    }
}

fn place_fields(local: usize, fields: &[(usize, Ty)]) -> MirPlace {
    MirPlace {
        local,
        projection: fields
            .iter()
            .map(|(field, ty)| MirProjection::Field(*field, *ty))
            .collect(),
    }
}

fn const_uint(value: u128, span: Span) -> MirOperand {
    let c = MirConst::try_from_uint(value, UintTy::Usize).expect("failed to build usize const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn const_u32(value: u128, span: Span) -> MirOperand {
    let c = MirConst::try_from_uint(value, UintTy::U32).expect("failed to build u32 const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn const_u8(value: u128, span: Span) -> MirOperand {
    let c = MirConst::try_from_uint(value, UintTy::U8).expect("failed to build u8 const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

fn fn_const_operand(fn_def: FnDef, generic_args: Vec<GenericArgKind>, span: Span) -> MirOperand {
    let fn_ty = Ty::from_rigid_kind(rustc_public_generative::rustc_public::ty::RigidTy::FnDef(
        fn_def,
        GenericArgs(generic_args),
    ));
    let c = MirConst::try_new_zero_sized(fn_ty).expect("failed to build fn const");
    MirOperand::Constant(ConstOperand {
        span,
        user_ty: None,
        const_: c,
    })
}

struct State {
    file_id: FileId,

    point_adt: AdtDef,
    human_adt: AdtDef,
    length_fn: FnDef,
    main_fn: FnDef,
    write_fn: FnDef,
}

unsafe impl Send for State {}
unsafe impl Sync for State {}

fn variant_idx(id: usize) -> VariantIdx {
    unsafe { std::mem::transmute::<usize, VariantIdx>(id) }
}

impl CrateGeneratorState for State {
    fn hir_structure(ctx: HirStructureCtx) -> (Self, HirStructure) {
        // Generate fake source code that matches the MIR we'll emit
        let source_code = r#"
struct Point { x: usize, y: usize }
struct Human { age: u32, location: Point }

fn length(h: Human) -> usize {
    let x = h.location.x;
    let y = h.location.y;
    x * x + y * y
}

extern "C" {
    fn write(fd: usize, buf: *mut u8, count: usize) -> usize;
}

fn main() {
    let point = Point { x: 3, y: 4 };
    let human = Human { location: point, age: 30 };
    let len = length(human);
    let msg = [50u8, 53, 10];
    unsafe { write(1, msg.as_mut_ptr() as *mut u8, 3); }
}
"#;
        let file_id = ctx.add_custom_file("/tmp/point_length.rs", source_code);

        // Spans pointing to actual locations in the fake source code
        let point_span = ctx.span_in_file(file_id, 1, 40); // struct Point line
        let human_span = ctx.span_in_file(file_id, 41, 85); // struct Human line
        let length_span = ctx.span_in_file(file_id, 87, 170); // fn length
        let main_span = ctx.span_in_file(file_id, 230, 380); // fn main
        let write_span = ctx.span_in_file(file_id, 172, 228); // extern block

        let root_crate = ctx.root_crate_def_id();
        let foreign_mod =
            ctx.allocate_def_id(root_crate, rustc_public_generative::DefData::ForeignMod);

        let point_adt = AdtDef(ctx.allocate_def_id(
            root_crate,
            rustc_public_generative::DefData::TypeNs("Point".to_owned()),
        ));
        let human_adt = AdtDef(ctx.allocate_def_id(
            root_crate,
            rustc_public_generative::DefData::TypeNs("Human".to_owned()),
        ));
        let length_fn_def = FnDef(ctx.allocate_def_id(
            root_crate,
            rustc_public_generative::DefData::ValueNs("length".to_owned()),
        ));
        let main_fn_def = FnDef(ctx.allocate_def_id(
            root_crate,
            rustc_public_generative::DefData::ValueNs("main".to_owned()),
        ));
        let write_fn_def = FnDef(ctx.allocate_def_id(
            foreign_mod,
            rustc_public_generative::DefData::ValueNs("write".to_owned()),
        ));

        let x_field = ctx.allocate_def_id(
            point_adt.0,
            rustc_public_generative::DefData::ValueNs("x".to_owned()),
        );
        let y_field = ctx.allocate_def_id(
            point_adt.0,
            rustc_public_generative::DefData::ValueNs("y".to_owned()),
        );
        let location_field = ctx.allocate_def_id(
            human_adt.0,
            rustc_public_generative::DefData::ValueNs("location".to_owned()),
        );
        let age_field = ctx.allocate_def_id(
            human_adt.0,
            rustc_public_generative::DefData::ValueNs("age".to_owned()),
        );

        let usize_ty = || HirTy::usize_ty(main_span);
        let u32_ty = HirTy::unsigned_ty(UintTy::U32, main_span);
        let u8_ty = HirTy::unsigned_ty(UintTy::U8, main_span);
        let ptr_u8_mut = HirTy::new_ptr(u8_ty, Mutability::Mut, main_span);

        let point_ty = HirTy::adt(point_adt, vec![], main_span);
        let human_ty = HirTy::adt(human_adt, vec![], main_span);

        let hir_structure = HirStructure {
            root: HirModule {
                span: main_span,
                items: vec![
                    HirModuleItem::Adt {
                        name: "Point".to_string(),
                        id: point_adt,
                        kind: HirAdtKind::Struct {
                            fields: vec![
                                StructField {
                                    id: x_field,
                                    name: "x".to_owned(),
                                    ty: usize_ty(),
                                },
                                StructField {
                                    name: "y".to_owned(),
                                    id: y_field,
                                    ty: usize_ty(),
                                },
                            ],
                        },
                        span: point_span,
                    },
                    HirModuleItem::Adt {
                        name: "Human".to_string(),
                        id: human_adt,
                        kind: HirAdtKind::Struct {
                            fields: vec![
                                StructField {
                                    name: "age".to_owned(),
                                    id: age_field,
                                    ty: u32_ty,
                                },
                                StructField {
                                    name: "location".to_owned(),
                                    id: location_field,
                                    ty: point_ty,
                                },
                            ],
                        },
                        span: human_span,
                    },
                    HirModuleItem::Function {
                        name: "length".to_string(),
                        id: length_fn_def,
                        sig: FunctionSignature {
                            inputs: vec![human_ty],
                            output: usize_ty(),
                            abi: FunctionAbi::Rust,
                            is_unsafe: false,
                        },
                        span: length_span,
                    },
                    HirModuleItem::Function {
                        name: "main".to_string(),
                        id: main_fn_def,
                        sig: FunctionSignature {
                            inputs: vec![],
                            output: HirTy::new_tuple(vec![], main_span),
                            abi: FunctionAbi::Rust,
                            is_unsafe: false,
                        },
                        span: main_span,
                    },
                    HirModuleItem::ForeignMod {
                        id: foreign_mod,
                        items: vec![ForeignModItem::ForeignFunction {
                            name: "write".to_string(),
                            id: write_fn_def,
                            sig: FunctionSignature {
                                inputs: vec![usize_ty(), ptr_u8_mut, usize_ty()],
                                output: usize_ty(),
                                abi: FunctionAbi::C,
                                is_unsafe: true,
                            },
                            span: write_span,
                        }],
                    },
                ],
            },
        };

        (
            State {
                file_id,

                point_adt,
                human_adt,
                length_fn: length_fn_def,
                main_fn: main_fn_def,
                write_fn: write_fn_def,
            },
            hir_structure,
        )
    }

    fn emit_mir(&mut self, ctx: HirStructureCtx, def: DefId) -> Body {
        // Spans pointing to actual locations in the fake source code
        let length_fn_span: Span = ctx.span_in_file(self.file_id, 81, 111); // "fn length(h: Human) -> usize {"
        let let_x_span: Span = ctx.span_in_file(self.file_id, 112, 137); // "    let x = h.location.x;"
        let let_y_span: Span = ctx.span_in_file(self.file_id, 138, 163); // "    let y = h.location.y;"
        let return_expr_span: Span = ctx.span_in_file(self.file_id, 164, 181); // "    x * x + y * y"
        let main_fn_span: Span = ctx.span_in_file(self.file_id, 263, 482); // fn main
        // Body span covering the entire function
        let body_span: Span = ctx.span_in_file(self.file_id, 81, 183);

        let usize_ty = Ty::usize_ty();
        let u8_ty = Ty::unsigned_ty(UintTy::U8);
        let point_ty =
            Ty::from_rigid_kind(rustc_public_generative::rustc_public::ty::RigidTy::Adt(
                self.point_adt,
                GenericArgs(vec![]),
            ));
        let human_ty =
            Ty::from_rigid_kind(rustc_public_generative::rustc_public::ty::RigidTy::Adt(
                self.human_adt,
                GenericArgs(vec![]),
            ));
        let tuple_u8_3_ty = Ty::new_tuple(&[u8_ty, u8_ty, u8_ty]);
        let ptr_tuple_u8_3 = Ty::new_ptr(tuple_u8_3_ty, Mutability::Mut);
        let ptr_u8_mut = Ty::new_ptr(u8_ty, Mutability::Mut);

        if def == self.length_fn.0 {
            let locals = vec![
                MirLocalDecl {
                    ty: usize_ty,
                    span: length_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: human_ty,
                    span: length_fn_span,
                    mutability: Mutability::Not,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: let_x_span,
                    mutability: Mutability::Not,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: let_y_span,
                    mutability: Mutability::Not,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: return_expr_span,
                    mutability: Mutability::Not,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: return_expr_span,
                    mutability: Mutability::Not,
                },
            ];

            let x_ty = Ty::usize_ty();
            let y_ty = Ty::usize_ty();

            let blocks = vec![MirBasicBlock {
                statements: vec![
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(2),
                            Rvalue::Use(MirOperand::Move(place_fields(
                                1,
                                &[(1, point_ty), (0, x_ty)],
                            ))),
                        ),
                        span: let_x_span,
                    },
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(3),
                            Rvalue::Use(MirOperand::Move(place_fields(
                                1,
                                &[(1, point_ty), (1, y_ty)],
                            ))),
                        ),
                        span: let_y_span,
                    },
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(4),
                            Rvalue::BinaryOp(
                                rustc_public_generative::rustc_public::mir::BinOp::Mul,
                                MirOperand::Move(place(2)),
                                MirOperand::Move(place(2)),
                            ),
                        ),
                        span: return_expr_span,
                    },
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(5),
                            Rvalue::BinaryOp(
                                rustc_public_generative::rustc_public::mir::BinOp::Mul,
                                MirOperand::Move(place(3)),
                                MirOperand::Move(place(3)),
                            ),
                        ),
                        span: return_expr_span,
                    },
                    MirStatement {
                        kind: MirStatementKind::Assign(
                            place(0),
                            Rvalue::BinaryOp(
                                rustc_public_generative::rustc_public::mir::BinOp::Add,
                                MirOperand::Move(place(4)),
                                MirOperand::Move(place(5)),
                            ),
                        ),
                        span: return_expr_span,
                    },
                ],
                terminator: MirTerminator {
                    kind: TerminatorKind::Return,
                    span: return_expr_span,
                },
            }];

            Body::new(blocks, locals, 1, vec![], None, body_span)
        } else if def == self.main_fn.0 {
            let locals = vec![
                MirLocalDecl {
                    ty: Ty::new_tuple(&[]),
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: point_ty,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: human_ty,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: tuple_u8_3_ty,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: ptr_tuple_u8_3,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: ptr_u8_mut,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
                MirLocalDecl {
                    ty: usize_ty,
                    span: main_fn_span,
                    mutability: Mutability::Mut,
                },
            ];

            let blocks = vec![
                MirBasicBlock {
                    statements: vec![
                        MirStatement {
                            kind: MirStatementKind::Assign(
                                place(1),
                                Rvalue::Aggregate(
                                    rustc_public_generative::rustc_public::mir::AggregateKind::Adt(
                                        self.point_adt,
                                        variant_idx(0),
                                        GenericArgs(vec![]),
                                        None,
                                        None,
                                    ),
                                    vec![
                                        const_uint(3, main_fn_span),
                                        const_uint(4, main_fn_span),
                                    ],
                                ),
                            ),
                            span: main_fn_span,
                        },
                        MirStatement {
                            kind: MirStatementKind::Assign(
                                place(2),
                                Rvalue::Aggregate(
                                    rustc_public_generative::rustc_public::mir::AggregateKind::Adt(
                                        self.human_adt,
                                        variant_idx(0),
                                        GenericArgs(vec![]),
                                        None,
                                        None,
                                    ),
                                    vec![
                                        const_u32(30, main_fn_span),
                                        MirOperand::Move(place(1)),
                                    ],
                                ),
                            ),
                            span: main_fn_span,
                        },
                    ],
                    terminator: MirTerminator {
                        kind: TerminatorKind::Call {
                            func: fn_const_operand(self.length_fn, vec![], main_fn_span),
                            args: vec![MirOperand::Move(place(2))],
                            destination: place(3),
                            target: Some(1),
                            unwind: UnwindAction::Continue,
                        },
                        span: main_fn_span,
                    },
                },
                MirBasicBlock {
                    statements: vec![
                        MirStatement {
                            kind: MirStatementKind::Assign(
                                place(4),
                                Rvalue::Aggregate(
                                    rustc_public_generative::rustc_public::mir::AggregateKind::Tuple,
                                    vec![
                                        const_u8(50, main_fn_span),
                                        const_u8(53, main_fn_span),
                                        const_u8(10, main_fn_span),
                                    ],
                                ),
                            ),
                            span: main_fn_span,
                        },
                        MirStatement {
                            kind: MirStatementKind::Assign(
                                place(5),
                                Rvalue::AddressOf(
                                    RawPtrKind::Mut,
                                    place(4),
                                ),
                            ),
                            span: main_fn_span,
                        },
                        MirStatement {
                            kind: MirStatementKind::Assign(
                                place(6),
                                Rvalue::Cast(
                                    CastKind::PtrToPtr,
                                    MirOperand::Move(place(5)),
                                    ptr_u8_mut,
                                ),
                            ),
                            span: main_fn_span,
                        },
                    ],
                    terminator: MirTerminator {
                        kind: TerminatorKind::Call {
                            func: fn_const_operand(self.write_fn, vec![], main_fn_span),
                            args: vec![
                                const_uint(1, main_fn_span),
                                MirOperand::Move(place(6)),
                                const_uint(3, main_fn_span),
                            ],
                            destination: place(7),
                            target: Some(2),
                            unwind: UnwindAction::Continue,
                        },
                        span: main_fn_span,
                    },
                },
                MirBasicBlock {
                    statements: vec![],
                    terminator: MirTerminator {
                        kind: TerminatorKind::Return,
                        span: main_fn_span,
                    },
                },
            ];

            Body::new(blocks, locals, 0, vec![], None, main_fn_span)
        } else {
            panic!("unexpected def: {:?}", def);
        }
    }
}

fn main() {
    generate::<State>();
}
