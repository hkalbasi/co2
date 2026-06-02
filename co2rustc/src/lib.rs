#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;

mod detect;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use co2_ast::{
    PrettyConfig, PrettyPrint, PrettyPrinter, StatelessResolver, co2_test_symbol_name,
    co2_test_symbol_suffix,
};
use co2_driver_lib::{CompileMode, compile_co2_file};

pub use detect::{DetectResult, detect_co2};

pub fn main() -> std::process::ExitCode {
    main_with_args(std::env::args().collect())
}

pub fn main_with_args(args: Vec<String>) -> std::process::ExitCode {
    rustc_driver::install_ice_hook("https://github.com/HKalbasi/co2", |_| ());
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

    let is_test = args.iter().any(|arg| arg == "--test");
    let mode = if is_test {
        CompileMode::RUST_TEST
    } else {
        CompileMode::RUST
    };
    let temp_host = if is_test {
        match write_test_host(&co2_file) {
            Ok(path) => Some(path),
            Err(err) => {
                eprintln!("co2rustc: failed to write test host: {err}");
                return std::process::ExitCode::from(1);
            }
        }
    } else {
        None
    };
    let args = if let Some(temp_host) = &temp_host {
        let host = co2_file.with_extension("rs");
        let mut out = args
            .into_iter()
            .filter(|arg| arg != "--test")
            .map(|arg| {
                if std::path::Path::new(&arg) == host {
                    temp_host.display().to_string()
                } else {
                    arg
                }
            })
            .collect::<Vec<_>>();
        out.push("--crate-type=bin".to_owned());
        out
    } else {
        args
    };

    if let Err(payload) = std::panic::catch_unwind(|| compile_co2_file(mode, &co2_file, args)) {
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

fn write_test_host(co2_file: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let preprocessed = co2_preprocessor::preprocess(co2_file, &[]);
    let ast = co2_parser::parse_translation_unit_from_preprocessed(
        &co2_file.display().to_string(),
        &preprocessed,
        StatelessResolver::new(),
    );

    let mut source = String::from("#![feature(test)]\n#![language(co2)]\nextern crate test;\n");
    let mut tests = Vec::new();
    if let Some(ast) = ast {
        collect_tests_from_translation_unit(
            &ast.0,
            &mut Vec::new(),
            &root_module_dir(co2_file),
            &mut tests,
        );
        for test in &tests {
            let _ = writeln!(source, "unsafe extern \"Rust\" {{ fn {}(); }}", test.symbol);
            let _ = writeln!(
                source,
                "fn {}() {{ unsafe {{ {}(); }} }}",
                test.host_fn, test.symbol
            );
        }
    }
    source.push_str("fn main() {\n");
    source.push_str("    test::test_main_static(&[\n");
    for test in tests {
        let _ = writeln!(
            source,
            "        &test::TestDescAndFn {{ desc: test::TestDesc {{ name: test::StaticTestName({:?}), ignore: false, ignore_message: None, source_file: \"\", start_line: 0, start_col: 0, end_line: 0, end_col: 0, compile_fail: false, no_run: false, should_panic: test::ShouldPanic::No, test_type: test::TestType::UnitTest }}, testfn: test::StaticTestFn(|| test::assert_test_result({}())) }},",
            test.display_name, test.host_fn
        );
    }
    source.push_str("    ]);\n");
    source.push_str("}\n");

    let path = std::env::temp_dir().join(format!(
        "co2-test-host-{}-{}.rs",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::write(&path, source)?;
    Ok(path)
}

#[derive(Debug)]
struct Co2Test {
    display_name: String,
    symbol: String,
    host_fn: String,
}

fn collect_tests_from_translation_unit(
    ast: &co2_ast::TranslationUnit<StatelessResolver>,
    module_path: &mut Vec<String>,
    module_dir: &std::path::Path,
    tests: &mut Vec<Co2Test>,
) {
    for (item, _) in &ast.items {
        let co2_ast::Declaration::FunctionDefinition { signature, .. } = item else {
            continue;
        };
        let co2_ast::FunctionDefinitionSignature::Rust(sig) = signature else {
            continue;
        };
        if !sig.attrs.iter().any(|(attr, _)| attr.is_word("test")) {
            continue;
        }
        let name = sig.name.0.clone();
        let symbol = co2_test_symbol_name(module_path, &name);
        tests.push(Co2Test {
            display_name: co2_test_display_name(module_path, &name),
            host_fn: format!("__co2_host{}", co2_test_symbol_suffix(module_path, &name)),
            symbol,
        });
    }

    for (mod_item, _) in &ast.rust_mod_items {
        module_path.push(mod_item.name.0.clone());
        if let Some((tokens, end_span)) = &mod_item.inline_content {
            let source_name = format!("<inline module '{}'>", mod_item.name.0);
            let child = co2_parser::parse_translation_unit_from_tokens(
                tokens,
                &source_name,
                "",
                *end_span,
                StatelessResolver::new(),
            )
            .0;
            collect_tests_from_translation_unit(
                &child,
                module_path,
                &module_dir.join(&mod_item.name.0),
                tests,
            );
        } else if let Some(module_path_on_disk) =
            resolve_module_source(module_dir, &mod_item.name.0)
        {
            let preprocessed = co2_preprocessor::preprocess(&module_path_on_disk, &[]);
            let source_name = module_path_on_disk.to_string_lossy().into_owned();
            if let Some(child) = co2_parser::parse_translation_unit_from_preprocessed(
                &source_name,
                &preprocessed,
                StatelessResolver::new(),
            ) {
                collect_tests_from_translation_unit(
                    &child.0,
                    module_path,
                    &child_module_dir(&module_path_on_disk),
                    tests,
                );
            }
        }
        module_path.pop();
    }
}

fn co2_test_display_name(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        return name.to_owned();
    }
    let mut display = module_path.join("::");
    display.push_str("::");
    display.push_str(name);
    display
}

fn root_module_dir(source_path: &std::path::Path) -> std::path::PathBuf {
    source_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn child_module_dir(source_path: &std::path::Path) -> std::path::PathBuf {
    if source_path.file_stem().and_then(|stem| stem.to_str()) == Some("mod") {
        source_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    } else {
        source_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(source_path.file_stem().unwrap_or_default())
    }
}

fn resolve_module_source(
    module_dir: &std::path::Path,
    module_name: &str,
) -> Option<std::path::PathBuf> {
    let direct = module_dir.join(format!("{module_name}.co2"));
    if direct.is_file() {
        return Some(direct);
    }

    let nested = module_dir.join(module_name).join("mod.co2");
    if nested.is_file() {
        return Some(nested);
    }

    None
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
    let filename = co2_file.display().to_string();
    let Some(ast) = co2_parser::parse_translation_unit_from_preprocessed(
        &filename,
        &preprocessed,
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
