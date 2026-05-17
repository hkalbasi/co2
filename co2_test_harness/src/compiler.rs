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

pub fn build_compilers(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["build", "-q", "--locked", "-p", "co2-multicall"])
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
    for applet in ["co2rustc", "co2cc", "co2cargo", "co2miri"] {
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
    suite: crate::suite::Suite,
    mode: Mode,
    test: &TestCase,
    json_diagnostics: bool,
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

    let mut compile_flags = directive_args(test, "compile-flags")?;

    if suite == crate::suite::Suite::Debuginfo
        && !compile_flags.windows(2).any(|w| w == ["-C", "debuginfo=2"])
        && !compile_flags.iter().any(|s| s.contains("debuginfo="))
    {
        compile_flags.extend(["-C".to_owned(), "debuginfo=2".to_owned()]);
    }

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
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
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
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
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
            if json_diagnostics {
                cmd.env("CO2_FORCE_JSON_DIAGNOSTICS", "1");
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
