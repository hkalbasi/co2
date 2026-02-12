#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rustc_public_generative as rustc_gen;

use co2_hir_mir::{MirModule, parse_and_lower};

mod detect;
mod mir;
mod types;

use detect::{DetectResult, detect_co2};
use mir::build_item_mir_infos;
use types::build_items;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let co2_file = match detect_co2(&args) {
        DetectResult::Continue(exit_code) => {
            std::process::exit(exit_code);
        }
        DetectResult::Co2(file) => file,
    };

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

    let module = parse_and_lower(co2_file.to_string_lossy().into_owned(), leaked)
        .expect("failed to parse and lower co2");
    let module_for_items = module.clone();

    let file_id_cell: Arc<Mutex<Option<rustc_gen::FileId>>> = Arc::new(Mutex::new(None));
    let co2_path = co2_file.clone();
    let co2_src_for_ctx = leaked;

    rustc_gen::generate(
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
                debug_defined(&module, &defined);
                let file_id = file_id_cell.lock().unwrap().expect("missing registered file");
                build_item_mir_infos(&module, &deps, &defined, &ctx, file_id)
            }
        },
    );
}

fn debug_defined(module: &MirModule, defined: &rustc_gen::DefinedCrateInfo) {
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
}
