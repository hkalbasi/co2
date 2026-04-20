use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn main_with_args(args: Vec<String>) -> i32 {
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
                    eprintln!("co2cargo init failed: {}", e);
                    1
                }
            };
        } else {
            return run_cargo(&args[1..]);
        }
    }

    let subcommand = &args[0];
    let subcommand_args = &args[1..];

    if subcommand == "init" {
        match cargo_init(subcommand_args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("co2cargo init failed: {}", e);
                1
            }
        }
    } else {
        run_cargo(&args[..])
    }
}

fn run_cargo(args: &[String]) -> i32 {
    let mut cmd = Command::new("cargo");
    cmd.args(args);
    cmd.env("RUSTC", "co2rustc");

    let status = cmd.status().expect("failed to execute cargo");

    status.code().unwrap_or(1)
}

fn cargo_init(args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("init");
    cmd.args(args);

    println!("Running: cargo init {}", args.join(" "));

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run cargo init: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "cargo init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let project_dir = determine_project_dir(args)?;
    let is_lib = args.contains(&"--lib".to_string());

    fixup_project(&project_dir, is_lib)?;

    println!("Successfully initialized co2 crate at {}", project_dir.display());

    Ok(())
}

fn determine_project_dir(args: &[String]) -> Result<PathBuf, String> {
    for i in 0..args.len() {
        if !args[i].starts_with('-') && args[i] != "init" {
            return Ok(PathBuf::from(&args[i]));
        }
    }

    let cwd = env::current_dir().map_err(|e| format!("get current dir: {}", e))?;
    let name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("could not determine project name from current directory")?;

    Ok(cwd.join(name))
}

fn fixup_project(project_dir: &Path, is_lib: bool) -> Result<(), String> {
    let src_dir = if is_lib {
        project_dir.join("src").join("lib.rs")
    } else {
        project_dir.join("src").join("main.rs")
    };

    if !src_dir.exists() {
        return Err(format!("expected file {} does not exist", src_dir.display()));
    }

    let new_content = "#![feature(register_tool)]\n#![feature(custom_inner_attributes)]\n#![register_tool(co2)]\n#![co2::language]\n".to_owned();

    fs::write(&src_dir, new_content).map_err(|e| format!("write {}: {}", src_dir.display(), e))?;

    let co2_file = project_dir.join(if is_lib { "src/lib.co2" } else { "src/main.co2" });

    let co2_template = if is_lib {
        "fn add(a: i32, b: i32) -> i32 {\n    return a + b;\n}\n"
    } else {
        "fn main() {}\n"
    };

    fs::write(&co2_file, co2_template).map_err(|e| format!("write {}: {}", co2_file.display(), e))?;

    println!("Added #![co2::language] to {}", src_dir.display());
    println!("Created {}", co2_file.display());

    Ok(())
}
