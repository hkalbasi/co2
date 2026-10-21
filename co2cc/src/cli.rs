use std::path::Path;
use std::process::ExitCode;

use crate::args::parse_args;
use crate::compile::run_co2c;

fn print_help() {
    eprintln!(
        "\
Usage: co2cc [options] file...

Options:
  -E                   Preprocess only; output preprocessed source
  -c                   Compile to object file only
  -S                   Compile to assembly only
  -emit-llvm           Emit LLVM (with -S: IR text .ll, with -c: bitcode .bc)
  -o <file>            Write output to <file>
  -shared              Build a shared library
  -I <dir>             Add directory to include search path
  -iquote <dir>        Add directory to quote include search path
  -isystem <dir>       Add directory to system include search path
  -x <language>        No effect (accepted for compatibility)
  -D <macro>[=<val>]   Define macro
  -U <macro>           Undefine macro
  -include <file>      Include file before compilation
  -nostdinc            Do not search standard include paths
  -undef               Do not predefine system macros
  -std=<standard>      No effect (accepted for compatibility)
  -O<level>            Optimization level
  -g                   Generate debug information
  -fPIC                Generate position-independent code
  -flto[=full]           Enable link-time optimization (full LTO only);
                         maps to rustc -C lto=fat (required for
                         -fsanitize=cfi, like clang)
  -fno-lto               Disable link-time optimization
  -l <lib>             Link against library
  -L <dir>             Add library search path
  -Wl,<option>         Pass option to linker
  -Xlinker <option>    Pass option to linker
  -masm=<flavor>       Use given assembly style (att/intel) for -S output
  -march=<cpu>         Target CPU for codegen (e.g. native, haswell, x86-64-v3);
                       maps to rustc -C target-cpu=<cpu>
  -mtune=<cpu>         Accepted for compatibility (no codegen effect)
  -m<feature>          Enable x86 target feature (e.g. -mavx2, -mfma, -msse4.2);
                       maps to rustc -C target-feature=+<feature>
  -mno-<feature>       Disable x86 target feature
  -fsanitize=<list>      Enable sanitizers (e.g. address); maps to
                         rustc -Zsanitizer=<list>
  -fno-sanitize=<list>   Disable previously enabled sanitizers
  -ftime-report        Print timing report
  -MD                  Generate make dependency file
  -MMD                 Like -MD but ignore system headers
  -MP                  Add phony targets to dependency file
  -MF <file>           Write dependency output to <file>
  -MT <target>         Use <target> as the rule name in dependency file
  -MQ <target>         Like -MT but quote the target for make
  -dumpmachine         Print the target machine triple and exit"
    );
}

fn print_cc_version() -> ExitCode {
    let co2_ver = std::env::var("CO2_VERSION").unwrap_or_else(|_| "unknown".to_owned());
    println!("co2cc (CO2) {co2_ver}");
    println!("{}", env!("RUSTC_VERSION"));
    // We emit llvm version as clang to make meson happy.
    println!("clang version: {}", env!("LLVM_VERSION"));
    ExitCode::SUCCESS
}

fn print_dumpmachine() -> ExitCode {
    println!("{}", env!("HOST_TRIPLE"));
    ExitCode::SUCCESS
}

pub(crate) fn main_with_args(args: &[String]) -> ExitCode {
    if args.iter().skip(1).any(|a| a == "-dumpmachine") {
        return print_dumpmachine();
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        return print_cc_version();
    }

    rustc_driver::install_ice_hook("https://github.com/HKalbasi/co2", |_| ());

    if let Ok(cwd) = std::env::current_dir() {
        co2_ast::set_diagnostic_base_path(Some(cwd));
    }

    if args.iter().skip(1).any(|a| a == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }

    for arg in args.iter().skip(1) {
        if !arg.starts_with('-') && Path::new(arg).extension().is_some_and(|e| e == "co2") {
            eprintln!(
                "co2cc: error: {arg}: co2cc is a C compiler, use co2cargo or co2rustc for compiling co2 files"
            );
            return ExitCode::from(2);
        }
    }

    let cc_args = match parse_args(args) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("co2cc: {msg}");
            return ExitCode::from(2);
        }
    };

    if cc_args.emit_llvm && !cc_args.emit_asm_only && !cc_args.emit_obj_only {
        eprintln!("co2cc: error: -emit-llvm cannot be used when linking");
        return ExitCode::from(1);
    }

    for input in &cc_args.inputs {
        if input != Path::new("-") && !input.exists() {
            eprintln!(
                "co2cc: error: {}: Input file does not exist",
                input.display()
            );
            return ExitCode::from(2);
        }
    }

    if let Err(payload) = std::panic::catch_unwind(|| run_co2c(&cc_args)) {
        if co2_ast::is_diagnostic_abort(payload.as_ref()) {
            return ExitCode::from(5);
        }
        if let Some(msg) = payload.downcast_ref::<String>() {
            eprintln!("co2cc panic: {msg}");
        } else if let Some(msg) = payload.downcast_ref::<&str>() {
            eprintln!("co2cc panic: {msg}");
        } else {
            eprintln!("co2cc panic: non-string payload");
        }
        return ExitCode::from(101);
    }

    ExitCode::SUCCESS
}
