use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use tempfile::{Builder as TempDirBuilder, TempDir};

use crate::test_case::{Mode, TestCase, directive_args};

pub struct CompileResult {
    pub output: Output,
    pub exe_path: PathBuf,
    pub _temp: TempDir,
}

pub enum MiriRun {
    Ran(Output),
    Unavailable(String),
}

pub fn run_miri_test(root: &Path, mode: Mode, test: &TestCase) -> Result<MiriRun> {
    match mode {
        Mode::Co2 => run_co2_miri_test(root, test),
        Mode::C | Mode::Rust => {
            bail!("`run-miri` is currently supported only for `//@ mode: co2` tests")
        }
    }
}

fn run_co2_miri_test(root: &Path, test: &TestCase) -> Result<MiriRun> {
    if !cargo_miri_available() {
        return Ok(MiriRun::Unavailable(
            "cargo-miri not available (install miri: rustup component add miri)".to_owned(),
        ));
    }

    let temp = TempDirBuilder::new()
        .prefix("co2-ct-miri-")
        .tempdir()
        .context("failed to create temp dir for miri test artifacts")?;
    let project = temp.path().join("project");
    let src_dir = project.join("src");
    fs::create_dir(&project).context("failed to create miri test project")?;
    fs::create_dir(&src_dir).context("failed to create miri test src dir")?;

    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"co2_miri_test\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .context("failed to write miri test Cargo.toml")?;
    fs::write(src_dir.join("main.rs"), "#![language(co2)]\n")
        .context("failed to write co2 shim rust file")?;
    fs::copy(&test.path, src_dir.join("main.co2")).context("failed to copy co2 test source")?;

    let run_args = directive_args(test, "run-args")?;

    let mut cmd = Command::new(root.join("target").join("debug").join("co2cargo"));
    cmd.current_dir(&project).args(["miri", "run", "-q"]);
    cmd.env("CO2_UNDER_MIRI", "1");
    if !run_args.is_empty() {
        cmd.arg("--").args(run_args);
    }

    let compile_flags = directive_args(test, "compile-flags")?;
    if !compile_flags.is_empty() {
        cmd.env("RUSTFLAGS", compile_flags.join(" "));
    }

    let path_sep = if cfg!(windows) { ";" } else { ":" };
    let current_path = std::env::var("PATH").unwrap_or_default();
    let compiler_bin = root.join("target").join("debug");
    let merged_path = if current_path.is_empty() {
        compiler_bin.to_string_lossy().into_owned()
    } else {
        format!("{}{}{}", compiler_bin.display(), path_sep, current_path)
    };
    cmd.env("PATH", merged_path);

    Ok(MiriRun::Ran(
        cmd.output()
            .context("failed to execute co2cargo miri run")?,
    ))
}

fn cargo_miri_available() -> bool {
    Command::new("rustup")
        .args(["which", "cargo-miri"])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn build_compilers(root: &Path, coverage: bool) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["build", "--locked", "-p", "co2-multicall"]);

    if coverage {
        cmd.env("RUSTFLAGS", "-C instrument-coverage");
    }

    let status = cmd
        .status()
        .context("failed to run cargo build for co2-multicall")?;
    if !status.success() {
        bail!("building co2-multicall failed with status {status}");
    }
    ensure_compiler_links(&root.join("target").join("debug"))?;
    Ok(())
}

fn ensure_compiler_links(bin_dir: &Path) -> Result<()> {
    let multicall = bin_dir.join(exe_name("co2-multicall"));
    for applet in ["co2rustc", "co2rustdoc", "co2cc", "co2cargo", "co2miri"] {
        ensure_link(&multicall, &bin_dir.join(exe_name(applet)))?;
    }
    Ok(())
}

pub fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn ensure_link(source: &Path, target: &Path) -> Result<()> {
    if let Ok(existing) = fs::read_link(target)
        && (existing == source || existing == Path::new(source.file_name().unwrap_or_default()))
    {
        return Ok(());
    }

    if target.exists() || fs::symlink_metadata(target).is_ok() {
        let metadata = fs::symlink_metadata(target)
            .with_context(|| format!("failed to stat existing {}", target.display()))?;
        if metadata.file_type().is_dir() {
            bail!("refusing to replace directory {}", target.display());
        }
        fs::remove_file(target)
            .with_context(|| format!("failed to remove existing {}", target.display()))?;
    }

    create_symlink_or_copy(source, target)
}

#[cfg(unix)]
fn create_symlink_or_copy(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target).or_else(|_| {
        fs::copy(source, target)
            .with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source.display(),
                    target.display()
                )
            })
            .map(|_| ())
    })
}

#[cfg(windows)]
fn create_symlink_or_copy(source: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_file(source, target).or_else(|_| {
        fs::copy(source, target)
            .with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source.display(),
                    target.display()
                )
            })
            .map(|_| ())
    })
}

pub fn compile_test(
    root: &Path,
    mode: Mode,
    test: &TestCase,
    json_diagnostics: bool,
    coverage_dir: Option<&Path>,
    dump_mir: bool,
) -> Result<CompileResult> {
    let temp = TempDirBuilder::new()
        .prefix("co2-ct-")
        .tempdir()
        .context("failed to create temp dir for test artifacts")?;
    let temp_path = temp.path();

    let name = test
        .path
        .file_stem()
        .and_then(OsStr::to_str)
        .context("invalid test filename")?;
    let exe_path = temp_path.join(format!("{name}.bin"));

    let compile_flags = directive_args(test, "compile-flags")?;

    let output = match mode {
        Mode::C => {
            let test_dir = test.path.parent().context("test path has no parent")?;
            for entry in fs::read_dir(test_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let dest = temp_path.join(path.file_name().unwrap());
                    fs::copy(&path, &dest)?;
                }
            }
            let c_src = temp_path.join(test.path.file_name().context("missing C test filename")?);

            let mut cmd = Command::new(root.join("target").join("debug").join("co2cc"));
            cmd.arg(&c_src).arg("-o").arg(&exe_path).args(compile_flags);
            cmd.arg("-I").arg(test_dir);
            if dump_mir {
                cmd.env("RUSTFLAGS", "-Zdump-mir=all");
                cmd.arg("-Zdump-mir=all");
            }
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            if let Some(dir) = coverage_dir {
                cmd.env(
                    "LLVM_PROFILE_FILE",
                    dir.join(format!(
                        "compiler-{}-%p-%m.profraw",
                        test.path.file_stem().unwrap().to_str().unwrap()
                    )),
                );
            }
            cmd.output().context("failed to execute co2cc")?
        }
        Mode::Co2 => {
            let co2_src = temp_path.join(format!("{name}.co2"));
            fs::copy(&test.path, &co2_src).context("failed to copy co2 test source")?;

            let shim = temp_path.join(format!("{name}.rs"));
            fs::write(&shim, "#![language(co2)]\n")
                .context("failed to write co2 shim rust file")?;

            let mut cmd = Command::new(root.join("target").join("debug").join("co2rustc"));
            cmd.arg(&shim)
                .arg("--crate-type=bin")
                .arg("--edition=2024")
                .arg("-o")
                .arg(&exe_path)
                .args(compile_flags);
            if dump_mir {
                cmd.env("RUSTFLAGS", "-Zdump-mir=all");
                cmd.arg("-Zdump-mir=all");
            }
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            if let Some(dir) = coverage_dir {
                cmd.env(
                    "LLVM_PROFILE_FILE",
                    dir.join(format!(
                        "compiler-{}-%p-%m.profraw",
                        test.path.file_stem().unwrap().to_str().unwrap()
                    )),
                );
            }
            cmd.output().context("failed to execute co2rustc")?
        }
        Mode::Rust => {
            let rust_src = temp_path.join(
                test.path
                    .file_name()
                    .context("missing Rust test filename")?,
            );
            fs::copy(&test.path, &rust_src).context("failed to copy Rust test source")?;

            let mut cmd = Command::new(root.join("target").join("debug").join("co2rustc"));
            cmd.arg("--edition=2024")
                .arg(&rust_src)
                .arg("-o")
                .arg(&exe_path)
                .args(compile_flags);
            if dump_mir {
                cmd.env("RUSTFLAGS", "-Zdump-mir=all");
                cmd.arg("-Zdump-mir=all");
            }
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
            }
            if let Some(dir) = coverage_dir {
                cmd.env(
                    "LLVM_PROFILE_FILE",
                    dir.join(format!(
                        "compiler-{}-%p-%m.profraw",
                        test.path.file_stem().unwrap().to_str().unwrap()
                    )),
                );
            }
            cmd.output()
                .context("failed to execute co2rustc for Rust test")?
        }
    };

    Ok(CompileResult {
        output,
        exe_path,
        _temp: temp,
    })
}
