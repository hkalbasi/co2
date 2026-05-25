#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

mod detect;

use std::collections::HashMap;
use std::sync::Arc;

use co2_ast::{PrettyConfig, PrettyPrint, PrettyPrinter, StatelessResolver};
use co2_driver_lib::{CompileMode, compile_co2_file};

pub use detect::{DetectResult, detect_co2};

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    let (args, dump_ast_tree) = take_unpretty_ast_tree_flag(args);

    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        co2_ast::set_diagnostic_base_path(Some(std::path::PathBuf::from(manifest_dir)));
    }

    co2_ast::set_force_json_diagnostics(rustc_requests_json_diagnostics(&args));

    let args = maybe_force_json_diagnostics(args);
    let co2_file = match detect_co2(&args) {
        DetectResult::Continue(exit_code) => {
            return exit_code;
        }
        DetectResult::Co2(file) => file,
    };

    if dump_ast_tree {
        return dump_ast_tree_for_file(&co2_file);
    }

    if let Err(payload) =
        std::panic::catch_unwind(|| compile_co2_file(CompileMode::RUST, &co2_file, args))
    {
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return std::process::ExitCode::from(5);
        }
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2rustc panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2rustc panic: {msg}");
        } else {
            eprintln!("co2rustc panic: non-string payload");
        }
        return std::process::ExitCode::from(101);
    }

    std::process::ExitCode::SUCCESS
}

fn take_unpretty_ast_tree_flag(args: Vec<String>) -> (Vec<String>, bool) {
    let mut filtered = Vec::with_capacity(args.len());
    let mut dump_ast_tree = false;
    let mut idx = 0;

    while idx < args.len() {
        if args[idx] == "-Z"
            && args
                .get(idx + 1)
                .is_some_and(|arg| arg == "unpretty=ast-tree")
        {
            dump_ast_tree = true;
            idx += 2;
            continue;
        }

        if args[idx] == "-Zunpretty=ast-tree" {
            dump_ast_tree = true;
            idx += 1;
            continue;
        }

        filtered.push(args[idx].clone());
        idx += 1;
    }

    (filtered, dump_ast_tree)
}

fn dump_ast_tree_for_file(co2_file: &std::path::Path) -> std::process::ExitCode {
    let preprocessed = co2_preprocessor::preprocess(co2_file, &[]);
    let src_static: &'static str = Box::leak(preprocessed.normalized.to_string().into_boxed_str());
    let filename = co2_file.display().to_string();
    let Some(ast) = co2_parser::parse_translation_unit(
        &filename,
        src_static,
        Some(&preprocessed),
        StatelessResolver::new(),
    ) else {
        return std::process::ExitCode::from(5);
    };

    let cwd = std::env::current_dir().ok();
    let file_names: HashMap<co2_ast::FileId, String> = preprocessed
        .files()
        .iter()
        .map(|(id, file)| {
            let pretty = cwd
                .as_deref()
                .and_then(|cwd| file.path.strip_prefix(cwd).ok())
                .map_or_else(
                    || file.path.display().to_string(),
                    |relative| relative.display().to_string(),
                );
            (*id, pretty)
        })
        .collect();
    let file_names = Arc::new(file_names);
    let config = PrettyConfig {
        indent: 2,
        show_file_name: true,
        file_name_for_id: Some(Arc::new({
            let file_names = Arc::clone(&file_names);
            move |file_id| {
                file_names
                    .get(&file_id)
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned())
            }
        })),
    };
    let mut pp = PrettyPrinter::new(&config);
    ast.0.pretty_print(&mut pp);
    print!("{}", pp.finish());
    std::process::ExitCode::SUCCESS
}

fn maybe_force_json_diagnostics(mut args: Vec<String>) -> Vec<String> {
    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some()
        && !args.iter().any(|arg| arg == "--error-format=json")
    {
        args.push("--error-format=json".to_owned());
    }
    args
}

fn rustc_requests_json_diagnostics(args: &[String]) -> bool {
    args.iter().enumerate().any(|(idx, arg)| {
        arg == "--error-format=json"
            || (arg == "--error-format" && args.get(idx + 1).is_some_and(|value| value == "json"))
    })
}
