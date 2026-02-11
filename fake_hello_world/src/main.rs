#![feature(rustc_private)]

use rustc_public_generative as gen;

fn main() {
    gen::generate(|ctx, deps| {
        let file = ctx.add_custom_file("<generated>", "fake hello world");
        let span = ctx.span(file, 0, 1);

        let std_env_args = deps.std_env_args.expect("std::env::args missing");
        let iter_nth = deps.iter_nth.expect("Iterator::nth missing");
        let option_unwrap = deps.option_unwrap.expect("Option::unwrap missing");
        let result_unwrap = deps.result_unwrap.expect("Result::unwrap missing");
        let cstring_new = deps.cstring_new.expect("CString::new missing");
        let cstring_into_raw = deps.cstring_into_raw.expect("CString::into_raw missing");

        let args_ty_id = deps.std_env_args_ty.expect("std::env::Args type missing");
        let string_ty_id = deps.string_ty.expect("String type missing");
        let cstring_ty_id = deps.cstring_ty.expect("CString type missing");
        let nul_error_ty_id = deps.nul_error_ty.expect("NulError type missing");
        let option_ty_id = deps.option_ty.expect("Option type missing");
        let result_ty_id = deps.result_ty.expect("Result type missing");

        let args_ty = gen::MirTy::Adt { id: args_ty_id, args: vec![] };
        let string_ty = gen::MirTy::Adt { id: string_ty_id, args: vec![] };
        let cstring_ty = gen::MirTy::Adt { id: cstring_ty_id, args: vec![] };
        let nul_error_ty = gen::MirTy::Adt { id: nul_error_ty_id, args: vec![] };
        let option_string_ty = gen::MirTy::Adt {
            id: option_ty_id,
            args: vec![string_ty.clone()],
        };
        let result_cstring_ty = gen::MirTy::Adt {
            id: result_ty_id,
            args: vec![cstring_ty.clone(), nul_error_ty.clone()],
        };

        let args_ref_ty = gen::MirTy::Ref { mutability: gen::MirMutability::Mut, to: Box::new(args_ty.clone()) };
        let cstring_ref_ty = gen::MirTy::Ref { mutability: gen::MirMutability::Not, to: Box::new(cstring_ty.clone()) };

        let ptr_i8_mut = gen::MirTy::Ptr { mutability: gen::MirMutability::Mut, to: Box::new(gen::MirTy::I8) };
        let ptr_i8_const = gen::MirTy::Ptr { mutability: gen::MirMutability::Not, to: Box::new(gen::MirTy::I8) };

        let mut next_fn_id = 1_000_000u64;
        let mut fresh_fn = || {
            let id = gen::FunctionId::new(next_fn_id);
            next_fn_id += 1;
            id
        };

        let write_id = fresh_fn();
        let fopen_id = fresh_fn();
        let fread_id = fresh_fn();
        let fclose_id = fresh_fn();
        let malloc_id = fresh_fn();
        let free_id = fresh_fn();

        let foreign_functions = vec![
            gen::ForeignFunctionInfo {
                name: "write".to_string(),
                inputs: vec![gen::MirTy::I32, ptr_i8_mut.clone(), gen::MirTy::Usize],
                output: gen::MirTy::Isize,
                id: write_id,
            },
            gen::ForeignFunctionInfo {
                name: "fopen".to_string(),
                inputs: vec![ptr_i8_mut.clone(), ptr_i8_const.clone()],
                output: ptr_i8_mut.clone(),
                id: fopen_id,
            },
            gen::ForeignFunctionInfo {
                name: "fread".to_string(),
                inputs: vec![
                    ptr_i8_mut.clone(),
                    gen::MirTy::Usize,
                    gen::MirTy::Usize,
                    ptr_i8_mut.clone(),
                ],
                output: gen::MirTy::Usize,
                id: fread_id,
            },
            gen::ForeignFunctionInfo {
                name: "fclose".to_string(),
                inputs: vec![ptr_i8_mut.clone()],
                output: gen::MirTy::I32,
                id: fclose_id,
            },
            gen::ForeignFunctionInfo {
                name: "malloc".to_string(),
                inputs: vec![gen::MirTy::Usize],
                output: ptr_i8_mut.clone(),
                id: malloc_id,
            },
            gen::ForeignFunctionInfo {
                name: "free".to_string(),
                inputs: vec![ptr_i8_mut.clone()],
                output: gen::MirTy::Unit,
                id: free_id,
            },
        ];

        let body = gen::MirBody {
            locals: vec![
                gen::MirLocalDecl {
                    ty: gen::MirTy::Unit,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: gen::MirTy::Isize,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: args_ty.clone(),
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: args_ref_ty.clone(),
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: option_string_ty,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: string_ty.clone(),
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: result_cstring_ty,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: cstring_ty.clone(),
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: cstring_ref_ty,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: ptr_i8_mut.clone(),
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: ptr_i8_mut.clone(),
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: ptr_i8_mut.clone(),
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: gen::MirTy::Usize,
                    mutability: gen::MirMutability::Mut,
                    span,
                    name: None,
                },
                gen::MirLocalDecl {
                    ty: gen::MirTy::I32,
                    mutability: gen::MirMutability::Not,
                    span,
                    name: None,
                },
            ],
            arg_count: 0,
            blocks: vec![
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: std_env_args,
                            args: vec![],
                        }),
                        args: vec![],
                        destination: Some((
                            gen::MirPlace {
                                local: 2,
                                projection: vec![],
                            },
                            1,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![gen::MirStatement::Assign(
                        gen::MirPlace {
                            local: 3,
                            projection: vec![],
                        },
                        gen::MirRvalue::Ref {
                            mutability: gen::MirMutability::Mut,
                            place: gen::MirPlace {
                                local: 2,
                                projection: vec![],
                            },
                        },
                    )],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: iter_nth,
                            args: vec![args_ty.clone()],
                        }),
                        args: vec![
                            gen::MirOperand::Move(gen::MirPlace {
                                local: 3,
                                projection: vec![],
                            }),
                            gen::MirOperand::Const(gen::MirConst::Usize(1)),
                        ],
                        destination: Some((
                            gen::MirPlace {
                                local: 4,
                                projection: vec![],
                            },
                            2,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: option_unwrap,
                            args: vec![string_ty.clone()],
                        }),
                        args: vec![gen::MirOperand::Move(gen::MirPlace {
                            local: 4,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 5,
                                projection: vec![],
                            },
                            3,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: cstring_new,
                            args: vec![string_ty.clone()],
                        }),
                        args: vec![gen::MirOperand::Move(gen::MirPlace {
                            local: 5,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 6,
                                projection: vec![],
                            },
                            4,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: result_unwrap,
                            args: vec![cstring_ty.clone(), nul_error_ty.clone()],
                        }),
                        args: vec![gen::MirOperand::Move(gen::MirPlace {
                            local: 6,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 7,
                                projection: vec![],
                            },
                            5,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: cstring_into_raw,
                            args: vec![],
                        }),
                        args: vec![gen::MirOperand::Move(gen::MirPlace {
                            local: 7,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 9,
                                projection: vec![],
                            },
                            6,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: fopen_id,
                            args: vec![],
                        }),
                        args: vec![
                            gen::MirOperand::Copy(gen::MirPlace {
                                local: 9,
                                projection: vec![],
                            }),
                            gen::MirOperand::Const(gen::MirConst::ByteStr(b"r\0".to_vec())),
                        ],
                        destination: Some((
                            gen::MirPlace {
                                local: 10,
                                projection: vec![],
                            },
                            7,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: malloc_id,
                            args: vec![],
                        }),
                        args: vec![gen::MirOperand::Const(gen::MirConst::Usize(4096))],
                        destination: Some((
                            gen::MirPlace {
                                local: 11,
                                projection: vec![],
                            },
                            8,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: fread_id,
                            args: vec![],
                        }),
                        args: vec![
                            gen::MirOperand::Copy(gen::MirPlace {
                                local: 11,
                                projection: vec![],
                            }),
                            gen::MirOperand::Const(gen::MirConst::Usize(1)),
                            gen::MirOperand::Const(gen::MirConst::Usize(4096)),
                            gen::MirOperand::Copy(gen::MirPlace {
                                local: 10,
                                projection: vec![],
                            }),
                        ],
                        destination: Some((
                            gen::MirPlace {
                                local: 12,
                                projection: vec![],
                            },
                            9,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: write_id,
                            args: vec![],
                        }),
                        args: vec![
                            gen::MirOperand::Const(gen::MirConst::I32(1)),
                            gen::MirOperand::Copy(gen::MirPlace {
                                local: 11,
                                projection: vec![],
                            }),
                            gen::MirOperand::Copy(gen::MirPlace {
                                local: 12,
                                projection: vec![],
                            }),
                        ],
                        destination: Some((
                            gen::MirPlace {
                                local: 1,
                                projection: vec![],
                            },
                            10,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: fclose_id,
                            args: vec![],
                        }),
                        args: vec![gen::MirOperand::Copy(gen::MirPlace {
                            local: 10,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 13,
                                projection: vec![],
                            },
                            11,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Call {
                        func: gen::MirOperand::Const(gen::MirConst::Fn {
                            id: free_id,
                            args: vec![],
                        }),
                        args: vec![gen::MirOperand::Copy(gen::MirPlace {
                            local: 11,
                            projection: vec![],
                        })],
                        destination: Some((
                            gen::MirPlace {
                                local: 0,
                                projection: vec![],
                            },
                            12,
                        )),
                    },
                },
                gen::MirBasicBlock {
                    statements: vec![],
                    terminator: gen::MirTerminator::Return,
                },
            ],
            span,
        };

        gen::CurrentCrateInfo {
            crate_name: "fake_hello_world".to_string(),
            foreign_functions,
            functions: vec![gen::FunctionInfo {
                name: "main".to_string(),
                body,
            }],
        }
    });
}
