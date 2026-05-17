#![feature(rustc_private)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args();
    let arg0_real = args.next().unwrap_or_else(|| "co2-multicall".to_owned());
    let arg0 = std::env::var("CO2_APPLET_OVERRIDE").unwrap_or(arg0_real);
    dispatch(&arg0, args)
}

fn dispatch(arg0: &str, args: impl IntoIterator<Item = String>) -> std::process::ExitCode {
    let mut args = args.into_iter();
    match applet_name(arg0) {
        Some("co2rustc") => {
            co2rustc::main_with_args(std::iter::once("co2rustc".to_owned()).chain(args).collect())
        }
        Some("co2cc") => {
            let cc_args: Vec<String> = std::iter::once("co2cc".to_owned()).chain(args).collect();
            co2cc::main_with_args(&cc_args)
        }
        Some("co2cargo") => {
            let args_vec: Vec<String> =
                std::iter::once("co2cargo".to_owned()).chain(args).collect();
            let code = co2cargo::main_with_args(&args_vec);
            std::process::ExitCode::from(code as u8)
        }
        Some("co2miri") => {
            co2miri::main_with_args(std::iter::once("co2miri".to_owned()).chain(args).collect())
        }
        Some("co2-multicall") => {
            if let Some("install") = args.next().as_deref() {
                install(args)
            } else {
                eprintln!("usage: co2-multicall install [target_dir]");
                std::process::ExitCode::from(2)
            }
        }
        _ => {
            if args.next().as_deref() == Some("install") {
                install(args)
            } else {
                eprintln!("unknown applet `{}`", Path::new(arg0).display());
                eprintln!(
                    "set up a symlink named `co2rustc`, `co2cc`, `co2cargo`, or `co2miri` pointing to `co2-multicall`"
                );
                std::process::ExitCode::from(2)
            }
        }
    }
}

fn applet_name(arg0: &str) -> Option<&str> {
    Path::new(arg0)
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| match name {
            "co2rustc" => Some("co2rustc"),
            "co2cc" => Some("co2cc"),
            "co2cargo" => Some("co2cargo"),
            "co2miri" => Some("co2miri"),
            "co2-multicall" => Some("co2-multicall"),
            _ => None,
        })
}

fn install(mut args: impl Iterator<Item = String>) -> std::process::ExitCode {
    let target_dir = args
        .next()
        .map_or_else(|| PathBuf::from("/usr/bin"), PathBuf::from);
    match try_install(&target_dir) {
        Ok(()) => {
            println!("Successfully installed to {}", target_dir.display());
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("co2-multicall install failed: {err}");
            std::process::ExitCode::from(1)
        }
    }
}

fn try_install(bin_dir: &Path) -> Result<(), String> {
    let current = std::env::var_os("CO2_RUN_SCRIPT").map_or_else(
        || {
            std::env::current_exe()
                .map_err(|err| format!("resolve current exe: {err}"))
                .unwrap()
        },
        PathBuf::from,
    );

    if !bin_dir.exists() {
        fs::create_dir_all(bin_dir)
            .map_err(|err| format!("create {}: {err}", bin_dir.display()))?;
    }

    let installed = bin_dir.join(exe_name("co2-multicall"));

    fs::copy(&current, &installed).map_err(|err| {
        format!(
            "copy {} -> {}: {err}",
            current.display(),
            installed.display()
        )
    })?;

    for applet in ["co2rustc", "co2cc", "co2cargo", "co2miri"] {
        let target = bin_dir.join(exe_name(applet));
        replace_symlink(Path::new("co2-multicall"), &target)?;
    }

    Ok(())
}

fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(unix)]
fn replace_symlink(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        let metadata = fs::symlink_metadata(target)
            .map_err(|err| format!("stat {}: {err}", target.display()))?;
        if metadata.file_type().is_dir() {
            return Err(format!(
                "refusing to replace directory {}",
                target.display()
            ));
        }
        fs::remove_file(target).map_err(|err| format!("remove {}: {err}", target.display()))?;
    }

    std::os::unix::fs::symlink(source, target).map_err(|err| {
        format!(
            "symlink {} -> {}: {err}",
            target.display(),
            source.display()
        )
    })
}

#[cfg(not(unix))]
fn replace_symlink(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("install currently only supports Unix-like systems".to_owned())
}
