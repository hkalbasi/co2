use std::{env, fs};

use chumsky_c_parser::{
    Field, Item, RustType, State,
    hir::{HirBody, HirCtxInterface},
};
use itertools::Itertools;

struct DumbHirCtx {
    state: State,
}

impl HirCtxInterface for DumbHirCtx {
    type Ty = RustType;
}

fn main() {
    let filename = env::args().nth(1).expect("Expected file argument");
    let src = fs::read_to_string(&filename)
        .expect("Failed to read file")
        .leak();

    let Some(state) = chumsky_c_parser::parse_items(filename, src) else {
        return;
    };

    println!(
        r#"
#![allow(nonstandard_style)]
use ::core::ffi::{{c_void as void, c_void as __builtin_va_list, c_void as __co2_anonymous}};
"#
    );

    let dumb_ctx = DumbHirCtx { state };

    for item in &dumb_ctx.state.items {
        match &item.0 {
            Item::Use(use_item) => {
                println!("use {};", use_item.path.iter().map(|x| &x.0).join("::"));
            }
            Item::Function { name, sig, body } => {
                let name = &name.0;
                let hir = HirBody::lower(body.clone(), &dumb_ctx, src);
                println!(
                    r#"fn {name}({}) -> {} {{
{}
}}"#,
                    sig.inputs
                        .iter()
                        .map(|input| format!("_: {}", input))
                        .join(", "),
                    sig.output,
                    hir.pretty_print()
                        .lines()
                        .map(|line| format!("    {line}"))
                        .join("\n"),
                );
            }
            Item::ExternFunction { name, sig } => {
                let name = &name.0;
                println!(
                    r#"unsafe extern "C" {{
    fn {name}({}) -> {};
}}"#,
                    sig.inputs
                        .iter()
                        .map(|input| format!("_: {}", input))
                        .join(", "),
                    sig.output,
                );
            }
            Item::TypeDef { name, value } => {
                let name = &name.0;
                println!("type {name} = {};", value.0);
            }
            Item::Static { name, ty } => {
                let name = &name.0;
                println!(
                    "static mut {name}: {} = unsafe {{ ::std::mem::zeroed() }};",
                    ty.0
                );
            }
            Item::Struct { name, fields } => {
                let name = &name.0;
                println!("struct {name} {{");
                for Field { name, ty } in fields {
                    let name = &name.0;
                    println!("    {name}: {},", ty.0);
                }
                println!("}}");
            }
        }
    }
}
