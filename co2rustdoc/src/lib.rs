#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rustc_session::{EarlyDiagCtxt, getopts};

pub fn main_with_args(args: Vec<String>) -> ExitCode {
    rustc_driver::install_ice_hook("https://github.com/HKalbasi/co2", |_| ());
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        co2_ast::set_diagnostic_base_path(Some(PathBuf::from(manifest_dir)));
    }

    let expanded_args = expand_rustdoc_args(&args);
    co2_ast::set_force_json_diagnostics(rustdoc_requests_json_diagnostics(&expanded_args));

    let Some(input_file) = rustdoc_input_file(&expanded_args) else {
        return rustdoc::run_with_callbacks(&raw_args(&args), &mut NoCallbacks);
    };
    if !is_co2_host_file(&input_file) {
        return rustdoc::run_with_callbacks(&raw_args(&args), &mut NoCallbacks);
    }

    let mut callbacks = Co2Callbacks::new(input_file.with_extension("co2"), &expanded_args);
    match std::panic::catch_unwind(AssertUnwindSafe(|| {
        rustdoc::run_with_callbacks(&raw_args(&args), &mut callbacks)
    })) {
        Ok(exit_code) => exit_code,
        Err(payload) => {
            if co2_ast::is_diagnostic_abort(payload.as_ref()) {
                return ExitCode::from(5);
            }
            if let Some(msg) = payload.downcast_ref::<String>() {
                eprintln!("co2rustdoc panic: {msg}");
            } else if let Some(msg) = payload.downcast_ref::<&str>() {
                eprintln!("co2rustdoc panic: {msg}");
            } else {
                eprintln!("co2rustdoc panic: non-string payload");
            }
            ExitCode::from(101)
        }
    }
}

struct NoCallbacks;

impl rustdoc::Callbacks for NoCallbacks {}

struct Co2Callbacks {
    inner: co2_driver_lib::Co2RustdocCallbacks,
}

impl Co2Callbacks {
    fn new(co2_file: PathBuf, rustc_args: &[String]) -> Self {
        Self {
            inner: co2_driver_lib::Co2RustdocCallbacks::new(&co2_file, rustc_args),
        }
    }
}

impl rustdoc::Callbacks for Co2Callbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        self.inner.config(config);
    }

    fn after_crate_root_parsing(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) {
        self.inner.after_crate_root_parsing(krate);
    }

    fn after_expansion(&mut self, tcx: rustc_middle::ty::TyCtxt<'_>) {
        self.inner.after_expansion(tcx);
    }
}

fn raw_args(args: &[String]) -> Vec<String> {
    std::iter::once(
        std::env::args()
            .next()
            .unwrap_or_else(|| "co2rustdoc".to_owned()),
    )
    .chain(args.iter().cloned())
    .collect()
}

fn expand_rustdoc_args(args: &[String]) -> Vec<String> {
    let early_dcx = EarlyDiagCtxt::new(rustc_session::config::ErrorOutputType::default());
    rustc_driver::args::arg_expand_all(&early_dcx, args)
}

fn rustdoc_input_file(args: &[String]) -> Option<PathBuf> {
    let mut options = getopts::Options::new();
    for option in rustdoc::opts() {
        option.apply(&mut options);
    }
    let matches = options.parse(args).ok()?;
    match matches.free.as_slice() {
        [input] => Some(PathBuf::from(input)),
        _ => None,
    }
}

fn rustdoc_requests_json_diagnostics(args: &[String]) -> bool {
    args.iter().enumerate().any(|(idx, arg)| {
        arg == "--error-format=json"
            || (arg == "--error-format" && args.get(idx + 1).is_some_and(|value| value == "json"))
    })
}

fn is_co2_host_file(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return false;
    }

    let Ok(source) = std::fs::read_to_string(path) else {
        return false;
    };
    let stripped = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    stripped.as_slice() == ["#![language(co2)]"] && path.with_extension("co2").is_file()
}
