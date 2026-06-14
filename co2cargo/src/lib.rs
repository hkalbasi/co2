use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
enum CargoInitError {
    RunCargoInit(std::io::Error),
    CargoInitFailed(String),
    CurrentDir(std::io::Error),
    MissingExpectedFile(PathBuf),
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for CargoInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunCargoInit(err) => write!(f, "failed to run cargo init: {err}"),
            Self::CargoInitFailed(stderr) => write!(f, "cargo init failed: {stderr}"),
            Self::CurrentDir(err) => write!(f, "get current dir: {err}"),
            Self::MissingExpectedFile(path) => {
                write!(f, "expected file {} does not exist", path.display())
            }
            Self::WriteFile { path, source } => write!(f, "write {}: {}", path.display(), source),
        }
    }
}

pub fn main_with_args(args: &[String]) -> i32 {
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        println!("co2cargo: A wrapper around cargo that supports CO2 language features.");
        println!();
        println!("Usage: co2cargo <subcommand> [args...]");
        println!();
        println!("CO2 Specific Commands:");
        println!("    init        Initialize a new co2 project in an existing directory.");
        println!("                Creates src/main.co2 and adds #![language(co2)] to src/main.rs.");
        println!();
        println!("Common Cargo Commands (forwarded with RUSTC=co2rustc):");
        println!("    build, b    Compile the current package");
        println!("    check, c    Analyze the current package and report errors");
        println!("    doc         Build this package's documentation");
        println!("    run, r      Run a binary or example of the local package");
        println!("    test, t     Run the tests");
        println!("    add         Add dependencies to a manifest file");
        println!("    miri        Run the project under miri via cargo-miri.");
        println!("                Requires cargo-miri to be installed.");
        println!();
        println!("All other cargo commands are supported and forwarded to cargo.");
        return 0;
    }

    if args.is_empty() {
        eprintln!("usage: co2cargo <subcommand> [args...]");
        return 1;
    }

    let first = &args[0];
    if first == "co2cargo" {
        if args.len() < 2 {
            eprintln!("usage: co2cargo <subcommand> [args...]");
            return 1;
        }
        let subcommand = &args[1];
        let subcommand_args = &args[2..];

        if subcommand == "init" {
            return match cargo_init(subcommand_args) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("co2cargo init failed: {e}");
                    1
                }
            };
        } else if subcommand == "doc" {
            return run_doc(subcommand_args);
        } else if subcommand == "miri" {
            return run_miri(subcommand_args);
        }
        return run_cargo(&args[1..]);
    }

    let subcommand = &args[0];
    let subcommand_args = &args[1..];

    if subcommand == "init" {
        match cargo_init(subcommand_args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("co2cargo init failed: {e}");
                1
            }
        }
    } else if subcommand == "doc" {
        run_doc(subcommand_args)
    } else if subcommand == "miri" {
        run_miri(subcommand_args)
    } else {
        run_cargo(args)
    }
}

fn run_miri(args: &[String]) -> i32 {
    let co2miri_path = bundled_applet_path("co2miri");

    let mut cmd = Command::new("cargo-miri");
    // cargo-miri is invoked by cargo as `cargo-miri miri <subcommand> [args...]`.
    // When we call it directly we must prepend `miri` ourselves.
    cmd.arg("miri");
    cmd.args(args);
    // cargo-miri looks for the miri binary via the MIRI env var.
    cmd.env("MIRI", &co2miri_path);
    // Forward RUSTC so cargo-miri's rustc queries also go through co2rustc.
    cmd.env("RUSTC", "co2rustc");
    cmd.env("RUSTDOC", bundled_applet_path("co2rustdoc"));
    cmd.env("CARGO_INCREMENTAL", "0");

    // The co2 bundle wrapper sets RUSTFLAGS="--sysroot=<cache> ..." so that
    // co2rustc/co2miri can find the stdlib. cargo-miri also passes --sysroot
    // explicitly on every compiler invocation, producing "Option 'sysroot'
    // given more than once". Strip our injected --sysroot so cargo-miri's
    // explicit value takes precedence.
    let rustflags_clean = rustflags_without_sysroot();
    if rustflags_clean.is_empty() {
        cmd.env_remove("RUSTFLAGS");
    } else {
        cmd.env("RUSTFLAGS", rustflags_clean);
    }

    let status = cmd.status().expect("failed to execute cargo-miri");
    status.code().unwrap_or(1)
}

fn run_doc(args: &[String]) -> i32 {
    let mut cmd = Command::new("cargo");
    cmd.arg("doc");
    cmd.args(args);
    cmd.env("RUSTC", "co2rustc");
    cmd.env("RUSTDOC", bundled_applet_path("co2rustdoc"));
    cmd.env("CARGO_INCREMENTAL", "0");

    let status = cmd.status().expect("failed to execute cargo");
    status.code().unwrap_or(1)
}

/// Remove `--sysroot <value>` / `--sysroot=<value>` tokens from RUSTFLAGS.
fn rustflags_without_sysroot() -> String {
    let flags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let mut out: Vec<&str> = Vec::new();
    let mut iter = flags.split_whitespace().peekable();
    while let Some(tok) = iter.next() {
        if tok == "--sysroot" {
            iter.next(); // skip the following path argument
        } else if tok.starts_with("--sysroot=") {
            // inline form — skip entirely
        } else {
            out.push(tok);
        }
    }
    out.join(" ")
}

fn run_cargo(args: &[String]) -> i32 {
    let mut cmd = Command::new("cargo");
    cmd.args(args);
    cmd.env("RUSTC", "co2rustc");
    cmd.env("RUSTDOC", bundled_applet_path("co2rustdoc"));
    cmd.env("CARGO_INCREMENTAL", "0");

    let status = cmd.status().expect("failed to execute cargo");

    status.code().unwrap_or(1)
}

fn bundled_applet_path(applet: &str) -> String {
    let self_path = std::env::current_exe().ok();
    let bin_dir = self_path.as_deref().and_then(|p| p.parent());

    bin_dir
        .map(|dir| dir.join(applet))
        .filter(|path| path.exists())
        .map_or_else(
            || applet.to_owned(),
            |path| path.to_string_lossy().into_owned(),
        )
}

fn cargo_init(args: &[String]) -> Result<(), CargoInitError> {
    let mut cmd = Command::new("cargo");
    cmd.arg("init");
    cmd.args(args);

    println!("Running: cargo init {}", args.join(" "));

    let output = cmd.output().map_err(CargoInitError::RunCargoInit)?;

    if !output.status.success() {
        return Err(CargoInitError::CargoInitFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let project_dir = determine_project_dir(args)?;
    let is_lib = args.contains(&"--lib".to_string());

    fixup_project(&project_dir, is_lib)?;

    println!(
        "Successfully initialized co2 crate at {}",
        project_dir.display()
    );

    Ok(())
}

fn determine_project_dir(args: &[String]) -> Result<PathBuf, CargoInitError> {
    for arg in args {
        if !arg.starts_with('-') && arg != "init" {
            return Ok(PathBuf::from(arg));
        }
    }

    env::current_dir().map_err(CargoInitError::CurrentDir)
}

fn fixup_project(project_dir: &Path, is_lib: bool) -> Result<(), CargoInitError> {
    let src_dir = if is_lib {
        project_dir.join("src").join("lib.rs")
    } else {
        project_dir.join("src").join("main.rs")
    };

    if !src_dir.exists() {
        return Err(CargoInitError::MissingExpectedFile(src_dir));
    }

    let new_content = "#![language(co2)]\n".to_owned();

    fs::write(&src_dir, new_content).map_err(|source| CargoInitError::WriteFile {
        path: src_dir.clone(),
        source,
    })?;

    let co2_file = project_dir.join(if is_lib {
        "src/lib.co2"
    } else {
        "src/main.co2"
    });

    let co2_template = if is_lib {
        r#"
mod tests {
    #include <assert.h>

    #[test]
    fn it_works() {
        i32 result = add(2, 2);
        assert(result == 4);
    }
}

fn add(a: i32, b: i32) -> i32 {
    return a + b;
}"#
    } else {
        r#"#include <stdio.h>

fn main() {
    printf("Hello world from CO2!\n");
}
"#
    };

    fs::write(&co2_file, co2_template).map_err(|source| CargoInitError::WriteFile {
        path: co2_file.clone(),
        source,
    })?;

    println!("Added #![language(co2)] to {}", src_dir.display());
    println!("Created {}", co2_file.display());

    Ok(())
}
