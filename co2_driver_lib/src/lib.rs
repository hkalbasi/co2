#![feature(rustc_private)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use co2_hir_mir::{MirModule, parse_and_lower};
use rustc_public_generative as rustc_gen;

mod mir;
mod types;

pub use types::CompileMode;

pub fn compile_co2_file(mode: CompileMode, co2_file: &Path) {
    let src = std::fs::read_to_string(co2_file).expect("failed to read co2 file");
    compile_co2_source(mode, co2_file.to_path_buf(), src, std::env::args().collect());
}

pub fn compile_co2_source(
    mode: CompileMode,
    source_path: PathBuf,
    source: String,
    rustc_args: Vec<String>,
) {
    let leaked: &'static str = Box::leak(source.into_boxed_str());

    let module = parse_and_lower(source_path.to_string_lossy().into_owned(), leaked)
        .expect("failed to parse and lower co2");
    let module_for_items = module.clone();

    let file_id_cell: Arc<Mutex<Option<rustc_gen::FileId>>> = Arc::new(Mutex::new(None));
    let co2_path = source_path.clone();
    let co2_src_for_ctx = leaked;

    rustc_gen::generate_with_args(
        rustc_args,
        {
            let file_id_cell = file_id_cell.clone();
            move |ctx, deps| {
                let file_id = ctx.add_custom_file(&co2_path, co2_src_for_ctx);
                *file_id_cell.lock().unwrap() = Some(file_id);
                types::build_items(&module_for_items, deps, mode)
            }
        },
        {
            let file_id_cell = file_id_cell.clone();
            move |ctx, deps, defined| {
                debug_defined(&module, &defined);
                let file_id = file_id_cell.lock().unwrap().expect("missing registered file");
                mir::build_item_mir_infos(&module, &deps, &defined, &ctx, file_id, mode)
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
