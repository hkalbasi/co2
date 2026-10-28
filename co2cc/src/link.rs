use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::args::LinkOutputKind;
use crate::rustc::{build_link_rustc_args, build_link_stub_rustc_args, shared_rust_flags};

pub(crate) fn current_invocation_path() -> Option<PathBuf> {
    std::env::args_os().next().map(PathBuf::from)
}

/// Resolve the C linker driver via the shared compiler detection
/// (`$CO2_CC`, else first of `cc`/`gcc`/`clang` on PATH).
fn resolve_c_compiler() -> String {
    co2_preprocessor::detect_c_compiler().unwrap_or_else(|| "cc".to_owned())
}

fn applet_command() -> Command {
    let exe = std::env::var_os("CO2_RUN_SCRIPT")
        .map(PathBuf::from)
        .or_else(current_invocation_path)
        .or_else(|| std::env::current_exe().ok())
        .expect("failed to locate co2cc executable");
    let mut cmd = Command::new(exe);
    cmd.env("CO2_APPLET_OVERRIDE", "co2rustc");
    cmd
}

pub(crate) fn link_objects(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    link_kind: LinkOutputKind,
    sanitizers: &[String],
    lto: bool,
) {
    if matches!(link_kind, LinkOutputKind::SharedLib) {
        link_shared_objects(objects, linker_args, output, sanitizers, lto);
        return;
    }

    if linker_args
        .iter()
        .any(|arg| Path::new(arg).extension().and_then(|ext| ext.to_str()) == Some("a"))
    {
        link_with_system_c(
            objects,
            linker_args,
            output,
            opt_level,
            debuginfo,
            sanitizers,
            lto,
        );
    } else {
        link_with_rust_stub(
            objects,
            linker_args,
            output,
            opt_level,
            debuginfo,
            sanitizers,
        );
    }
}

// TODO: We do some horrible things here, to not link rust std when std is not actually used.
// We should instead work with upstream to reduce binary size when std is not actually used,
// or drop this feature entirely and force a `#![yes_std]` when people want std in co2cc programs.
fn link_with_rust_stub(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    sanitizers: &[String],
) {
    let temp_dir = tempfile::Builder::new()
        .prefix(".co2cc-")
        .tempdir()
        .expect("failed to create temporary directory");

    let link_with_stub = |stub: &str| -> Option<std::process::Output> {
        let stub_path = temp_dir.path().join("co2c_link.rs");
        let _ = fs::write(&stub_path, stub);
        let rustc_link_args = build_link_rustc_args(
            &stub_path,
            objects,
            linker_args,
            output,
            opt_level,
            debuginfo,
            sanitizers,
        );
        let output = applet_command().args(&rustc_link_args).output().ok()?;
        let _ = fs::remove_file(&stub_path);
        Some(output)
    };

    let forward_output = |output: &std::process::Output| {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&output.stdout);
        let _ = std::io::stderr().write_all(&output.stderr);
    };

    // Try linking with the no_std stub first (produces smaller binaries).
    // TODO: This is invalid garbage.
    if let Some(ref output) = link_with_stub(CO2C_LINK_STUB) {
        if output.status.success() {
            forward_output(output);
            return;
        }
        // If the no_std link failed due to std::/alloc:: unresolved symbols,
        // retry with the std stub.
        let stderr = String::from_utf8_lossy(&output.stderr);
        if (stderr.contains("undefined symbol: std::")
            || stderr.contains("undefined symbol: <std::")
            || stderr.contains("reference to `std::")
            || stderr.contains("undefined symbol: alloc::")
            || stderr.contains("undefined symbol: <alloc::")
            || stderr.contains("reference to `alloc::")
            || stderr.contains("_Unwind_Resume"))
            && let Some(ref retry_output) = link_with_stub(CO2C_LINK_STD_STUB)
            && retry_output.status.success()
        {
            forward_output(retry_output);
            return;
        }
        forward_output(output);
    }
    let _ = fs::remove_dir_all(temp_dir.path());
    process::exit(1);
}

fn link_with_system_c(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    opt_level: Option<&str>,
    debuginfo: Option<u8>,
    sanitizers: &[String],
    lto: bool,
) {
    let temp_dir = tempfile::Builder::new()
        .prefix(".co2cc-")
        .tempdir()
        .expect("failed to create temporary directory");
    let cc_link_stub = temp_dir.path().join("co2c_cc_link.rs");
    let cc_link_stub_object = temp_dir.path().join("co2c_cc_link.o");
    fs::write(&cc_link_stub, CO2C_CC_LINK_STUB).expect("failed to write linker stub");

    let rustc_args = build_link_stub_rustc_args(&cc_link_stub, &cc_link_stub_object, debuginfo);
    let stub_status = applet_command()
        .args(&rustc_args)
        .status()
        .expect("failed to compile co2cc panic stub object");
    if !stub_status.success() {
        let _ = fs::remove_dir_all(temp_dir.path());
        process::exit(1);
    }

    let mut link_cmd = Command::new(resolve_c_compiler());
    if !sanitizers.is_empty() {
        link_cmd.arg(format!("-fsanitize={}", sanitizers.join(",")));
    }
    if lto {
        link_cmd.arg("-flto");
    }
    for object in objects {
        link_cmd.arg(object);
    }
    link_cmd.arg(&cc_link_stub_object);
    for arg in linker_args {
        link_cmd.arg(arg);
    }
    link_cmd.arg("-o");
    link_cmd.arg(output.unwrap_or_else(|| Path::new("a.out")));

    let link_output = link_cmd
        .output()
        .expect("failed to execute executable link step");
    if link_output.status.success() {
        return;
    }

    let cc_stderr = String::from_utf8_lossy(&link_output.stderr);
    if !needs_rust_runtime(&cc_stderr) {
        let _ = fs::remove_dir_all(temp_dir.path());
        eprintln!(
            "executable link failed with status {}\n{}",
            link_output.status, cc_stderr
        );
        process::exit(1);
    }

    let needs_full_std = cc_stderr.contains("undefined symbol: std::")
        || cc_stderr.contains("undefined symbol: <std::")
        || cc_stderr.contains("reference to `std::");
    let link_stub_src = if needs_full_std {
        CO2C_LINK_STD_STUB
    } else {
        CO2C_LINK_STUB
    };

    let rustc_link_stub = temp_dir.path().join("co2c_link.rs");
    fs::write(&rustc_link_stub, link_stub_src).expect("failed to write rustc linker stub");
    let rustc_link_args = build_link_rustc_args(
        &rustc_link_stub,
        objects,
        linker_args,
        output,
        opt_level,
        debuginfo,
        sanitizers,
    );
    let status = applet_command()
        .args(&rustc_link_args)
        .status()
        .expect("failed to execute co2rustc link step");
    if !status.success() {
        let _ = fs::remove_dir_all(temp_dir.path());
        process::exit(1);
    }
}

fn needs_rust_runtime(stderr: &str) -> bool {
    ["alloc::", "core::", "std::", "__rust", "_RN"]
        .iter()
        .any(|needle| stderr.contains(needle))
}

fn link_shared_objects(
    objects: &[PathBuf],
    linker_args: &[String],
    output: Option<&Path>,
    sanitizers: &[String],
    lto: bool,
) {
    let run_link = |rust_runtime: Option<&Path>| {
        let mut cmd = Command::new(resolve_c_compiler());
        cmd.arg("-shared");
        if !sanitizers.is_empty() {
            cmd.arg(format!("-fsanitize={}", sanitizers.join(",")));
        }
        if lto {
            cmd.arg("-flto");
        }
        for object in objects {
            cmd.arg(object);
        }
        if let Some(runtime) = rust_runtime {
            cmd.arg(runtime);
        }
        for arg in linker_args {
            cmd.arg(arg);
        }
        cmd.arg("-o");
        cmd.arg(output.unwrap_or_else(|| Path::new("a.out")));
        cmd.output()
            .expect("failed to execute shared library link step")
    };

    let first = run_link(None);
    if first.status.success() {
        return;
    }

    let cc_stderr = String::from_utf8_lossy(&first.stderr);
    if !needs_rust_runtime(&cc_stderr) {
        eprintln!("{cc_stderr}");
        process::exit(1);
    }

    // The objects reference the Rust runtime (alloc/core/std) but a plain C
    // shared link does not include it. Add a staticlib holding std; the linker
    // only pulls in referenced members, so unrelated C symbols stay holes.
    let temp_dir = tempfile::Builder::new()
        .prefix(".co2cc-")
        .tempdir()
        .expect("failed to create temporary directory");
    let Ok(runtime) = build_shared_runtime_archive(temp_dir.path()) else {
        eprintln!("{cc_stderr}");
        let _ = fs::remove_dir_all(temp_dir.path());
        process::exit(1);
    };

    let retry = run_link(Some(&runtime));
    if !retry.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&retry.stderr));
        let _ = fs::remove_dir_all(temp_dir.path());
        process::exit(1);
    }
}

fn build_shared_runtime_archive(temp_dir: &Path) -> Result<PathBuf, ()> {
    let stub_path = temp_dir.join("co2c_shared_runtime.rs");
    fs::write(&stub_path, CO2C_LINK_STD_STUB).map_err(|_| ())?;
    let archive = temp_dir.join("libco2c_shared_runtime.a");

    let mut rustc_args = vec![
        "--crate-name".to_owned(),
        "co2c_shared_runtime".to_owned(),
        "--crate-type=staticlib".to_owned(),
        "--edition=2024".to_owned(),
        "-o".to_owned(),
        archive.to_string_lossy().into_owned(),
        "-C".to_owned(),
        "opt-level=2".to_owned(),
        "-C".to_owned(),
        "panic=abort".to_owned(),
        "-C".to_owned(),
        "relocation-model=pic".to_owned(),
    ];
    rustc_args.extend(shared_rust_flags());
    rustc_args.push(stub_path.to_string_lossy().into_owned());

    let output = applet_command()
        .args(&rustc_args)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(());
    }

    Ok(archive)
}

const CO2C_CC_LINK_STUB: &str = r#"#![no_std]
#![no_main]

use core::ffi::{c_int, c_void};

unsafe extern "C" {
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize;
    fn exit(code: c_int) -> !;

    static mut stderr: *mut c_void;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        fwrite(c"co2cc panic\n".as_ptr().cast(), 1, 12, stderr);
        exit(134);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
"#;

const CO2C_LINK_STUB: &str = r#"#![no_std]
#![no_main]

use core::ffi::{c_int, c_void, c_char};

unsafe extern "C" {
    fn fprintf(stream: *mut c_void, fmt: *const c_char, ...) -> c_int;
    fn exit(code: c_int) -> !;
    fn getpid() -> c_int;

    static mut stderr: *mut c_void;
}


#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        let pid = getpid();

        if let Some(loc) = info.location() {
            let file = loc.file();

            fprintf(
                stderr,
                c"thread '<unnamed>' (%d) panicked at %.*s:%d:%d:\n".as_ptr(),
                pid,
                file.len() as c_int,
                file.as_ptr(),
                loc.line(),
                loc.column(),
            );
        } else {
            fprintf(
                stderr,
                c"thread '<unnamed>' (%d) panicked:\n".as_ptr(),
                pid,
            );
        }

        if let Some(msg) = info.message().as_str() {
            fprintf(
                stderr,
                c"%.*s\n".as_ptr(),
                msg.len() as c_int,
                msg.as_ptr(),
            );
        } else {
            fprintf(
                stderr,
                c"<non-string panic>\n".as_ptr(),
            );
        }

        fprintf(
            stderr,
            c"%s\n".as_ptr(),
            c"thread caused non-unwinding panic. aborting.".as_ptr(),
        );

        exit(134);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}
"#;

const CO2C_LINK_STD_STUB: &str = r"#![no_main]
";
