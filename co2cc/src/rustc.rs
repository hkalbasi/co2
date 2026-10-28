use std::fs;
use std::path::{Path, PathBuf};

use crate::args::CcArgs;

pub(crate) fn sanitize_crate_name(stem: &str) -> String {
    let sanitized: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "co2c_out".to_owned()
    } else {
        sanitized
    }
}

fn crate_stem(input: &Path) -> String {
    input
        .file_stem()
        .and_then(|s| s.to_str())
        .map_or_else(|| "co2c_out".to_owned(), sanitize_crate_name)
}

fn push_common_codegen_args(
    rustc_args: &mut Vec<String>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    pic: bool,
    arch: &CcArgs,
) {
    if let Some(level) = opt_level {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("opt-level={level}"));
    }

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

    if pic {
        rustc_args.push("-C".to_owned());
        rustc_args.push("relocation-model=pic".to_owned());
    }

    // co2cc links panic=abort (see build_link_rustc_args), so objects and
    // assembly must use the same strategy.
    rustc_args.push("-C".to_owned());
    rustc_args.push("panic=abort".to_owned());

    push_arch_rustc_args(rustc_args, arch);
    push_sanitizer_rustc_args(rustc_args, arch);
    push_lto_rustc_args(rustc_args, arch);
    rustc_args.extend(shared_rust_flags());
}

fn build_rustc_emit_args(
    input: &Path,
    output: Option<&Path>,
    emit: &str,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    pic: bool,
    asm_flavor: Option<&str>,
    arch: &CcArgs,
) -> Vec<String> {
    let mut rustc_args = vec![
        "rustc".to_owned(),
        "--crate-name".to_owned(),
        crate_stem(input),
        "--crate-type=staticlib".to_owned(),
        "--edition=2024".to_owned(),
        emit.to_owned(),
    ];

    if let Some(out) = output {
        rustc_args.push("-o".to_owned());
        rustc_args.push(out.to_string_lossy().into_owned());
    }

    rustc_args.push(no_main_host_file().to_string_lossy().into_owned());
    if let Some(flavor) = asm_flavor {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("llvm-args=-x86-asm-syntax={flavor}"));
    }
    push_common_codegen_args(&mut rustc_args, opt_level, debuginfo, pic, arch);
    rustc_args
}

pub(crate) fn build_rustc_object_args(
    input: &Path,
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    pic: bool,
    emit_llvm: bool,
    arch: &CcArgs,
) -> Vec<String> {
    build_rustc_emit_args(
        input,
        output,
        if emit_llvm {
            "--emit=llvm-bc"
        } else {
            "--emit=obj"
        },
        opt_level,
        debuginfo,
        pic,
        None,
        arch,
    )
}

pub(crate) fn build_rustc_asm_args(
    input: &Path,
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    pic: bool,
    asm_flavor: Option<&str>,
    emit_llvm: bool,
    arch: &CcArgs,
) -> Vec<String> {
    build_rustc_emit_args(
        input,
        output,
        if emit_llvm {
            "--emit=llvm-ir"
        } else {
            "--emit=asm"
        },
        opt_level,
        debuginfo,
        pic,
        asm_flavor,
        arch,
    )
}

fn no_main_host_file() -> PathBuf {
    let path = std::env::temp_dir().join(format!("co2cc-no-main-{}.rs", std::process::id()));
    let _ = fs::write(&path, "#![no_main]\n");
    path
}

pub(crate) fn build_link_stub_rustc_args(
    link_stub: &Path,
    output: &Path,
    debuginfo: Option<u8>,
) -> Vec<String> {
    let mut rustc_args = vec![
        "--crate-name".to_owned(),
        "co2c_link_stub".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "--emit=obj".to_owned(),
        "-o".to_owned(),
        output.to_string_lossy().into_owned(),
        "-C".to_owned(),
        "opt-level=2".to_owned(),
        "-C".to_owned(),
        format!("debuginfo={}", debuginfo.unwrap_or(0)),
        "-C".to_owned(),
        "panic=abort".to_owned(),
    ];
    rustc_args.extend(shared_rust_flags());
    rustc_args.push(link_stub.to_string_lossy().into_owned());
    rustc_args
}

pub(crate) fn build_link_rustc_args(
    link_stub: &Path,
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    sanitizers: &[String],
) -> Vec<String> {
    let mut rustc_args = vec![
        "--crate-name".to_owned(),
        "co2c_link".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "-o".to_owned(),
        output
            .unwrap_or_else(|| Path::new("a.out"))
            .to_string_lossy()
            .into_owned(),
    ];

    for object in objects {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("link-arg={}", object.to_string_lossy()));
    }
    rustc_args.push("-C".to_owned());
    rustc_args.push("link-arg=-lc".to_owned());
    for arg in linker_args {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("link-arg={arg}"));
    }

    if let Some(level) = opt_level {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("opt-level={level}"));
    }

    let dbg = debuginfo.unwrap_or(0);
    rustc_args.push("-C".to_owned());
    rustc_args.push(format!("debuginfo={dbg}"));

    rustc_args.push("-C".to_owned());
    rustc_args.push("panic=abort".to_owned());

    // CFI is call-site instrumentation with no link-time runtime: forwarding
    // it would only force LTO onto the link stub, so it is dropped here.
    // (`-flto` is likewise compile-only: the link inputs are prebuilt
    // objects, so LTO cannot reach into them.)
    let link_sanitizers: Vec<&str> = sanitizers
        .iter()
        .filter(|s| s.as_str() != "cfi")
        .map(String::as_str)
        .collect();
    if !link_sanitizers.is_empty() {
        rustc_args.push("-Z".to_owned());
        rustc_args.push(format!("sanitizer={}", link_sanitizers.join(",")));
        // As on the compile path, non-`address` sanitizers need the ABI
        // mismatch allowance against prebuilt std.
        rustc_args.push("-C".to_owned());
        rustc_args.push("unsafe-allow-abi-mismatch=sanitizer".to_owned());
        // The sanitizer runtimes reference libgcc (unwinding), libm, libdl
        // and libpthread, which the no_std link would otherwise omit
        // (`-nodefaultlibs` + only `-lc`).
        for lib in ["-lm", "-lgcc_s", "-lgcc", "-ldl", "-lpthread"] {
            rustc_args.push("-C".to_owned());
            rustc_args.push(format!("link-arg={lib}"));
        }
    }

    rustc_args.extend(shared_rust_flags());
    rustc_args.push(link_stub.to_string_lossy().into_owned());
    rustc_args
}

pub(crate) fn shared_rust_flags() -> Vec<String> {
    let mut flags = std::env::var("RUSTFLAGS")
        .ok()
        .map(|flags| {
            flags
                .split_ascii_whitespace()
                .map(std::borrow::ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if std::env::var_os("CO2_FORCE_JSON_DIAGNOSTICS").is_some()
        && !flags.iter().any(|arg| arg == "--error-format=json")
    {
        flags.push("--error-format=json".to_owned());
    }

    flags
}

/// Append rustc codegen args for `-march=`/`-m<feature>` flags.
fn push_arch_rustc_args(rustc_args: &mut Vec<String>, args: &CcArgs) {
    if let Some(cpu) = &args.target_cpu {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("target-cpu={cpu}"));
    }
    if !args.target_features.is_empty() {
        rustc_args.push("-C".to_owned());
        rustc_args.push(format!("target-feature={}", args.target_features.join(",")));
    }
}

/// Append rustc codegen args for `-fsanitize=` flags.
fn push_sanitizer_rustc_args(rustc_args: &mut Vec<String>, args: &CcArgs) {
    if args.sanitizers.is_empty() {
        return;
    }
    rustc_args.push("-Z".to_owned());
    rustc_args.push(format!("sanitizer={}", args.sanitizers.join(",")));
    // Every sanitizer except `address` changes the Rust ABI, so compiling
    // against prebuilt (uninstrumented) std needs this allowance. It is a
    // no-op for `address`, so it is pushed unconditionally.
    rustc_args.push("-C".to_owned());
    rustc_args.push("unsafe-allow-abi-mismatch=sanitizer".to_owned());
    if args.sanitizers.iter().any(|s| s == "cfi") {
        // `-Zsanitizer=cfi` requires a single codegen unit. (The LTO it
        // also requires comes from explicit `-flto`, like clang.)
        rustc_args.push("-C".to_owned());
        rustc_args.push("codegen-units=1".to_owned());
    }
}

/// Append rustc `-C lto=fat` for `-flto`/`-flto=full`.
fn push_lto_rustc_args(rustc_args: &mut Vec<String>, args: &CcArgs) {
    if args.lto {
        rustc_args.push("-C".to_owned());
        rustc_args.push("lto=fat".to_owned());
    }
}
