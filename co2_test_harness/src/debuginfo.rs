use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::compiler::CompileResult;
use crate::test_case::TestCase;

pub fn run_debuginfo_test(test: &TestCase, compile: &CompileResult) -> Result<()> {
    let commands: Vec<String> = test
        .directives
        .get("gdb-command")
        .cloned()
        .unwrap_or_default();

    let checks: Vec<String> = test
        .directives
        .get("gdb-check")
        .cloned()
        .unwrap_or_default();

    if commands.is_empty() && checks.is_empty() {
        return Ok(());
    }

    let breakpoint_lines: Vec<usize> = test
        .source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("#break"))
        .map(|(i, _)| i + 1)
        .collect();

    let gdb = find_gdb().context("GDB not found on PATH; required for debuginfo tests")?;
    let gdb_checks = checks.clone();
    run_gdb(
        test,
        compile,
        &gdb,
        &commands,
        &gdb_checks,
        &breakpoint_lines,
    )?;
    Ok(())
}

fn find_gdb() -> Result<String> {
    Command::new("gdb")
        .arg("--version")
        .output()
        .map_err(|_| anyhow::anyhow!("failed to run gdb --version"))
        .and_then(|o| {
            if o.status.success() {
                Ok("gdb".into())
            } else {
                anyhow::bail!("gdb --version exited with failure")
            }
        })
}

fn run_gdb(
    test: &TestCase,
    compile: &CompileResult,
    gdb: &str,
    commands: &[String],
    checks: &[String],
    breakpoint_lines: &[usize],
) -> Result<()> {
    use std::io::Write;

    let source_file = test.path.file_name().unwrap().to_string_lossy();

    let mut script = String::new();
    script.push_str("set print pretty off\n");
    script.push_str("set pagination off\n");
    script.push_str(&format!("file {}\n", compile.exe_path.display()));
    for line in breakpoint_lines {
        script.push_str(&format!("break {}:{}\n", source_file, line));
    }
    for cmd in commands {
        script.push_str(cmd);
        script.push('\n');
    }
    script.push_str("quit\n");

    let mut script_file = tempfile::NamedTempFile::with_prefix("gdb-script-")
        .context("failed to create GDB script tempfile")?;
    script_file
        .write_all(script.as_bytes())
        .context("failed to write GDB script")?;

    let output = Command::new(gdb)
        .args(["-quiet", "-batch", "-nx", "-x"])
        .arg(script_file.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .context("failed to execute GDB")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        bail!(
            "GDB execution failed\nstdout:\n{}\nstderr:\n{}",
            stdout,
            stderr
        );
    }

    check_debugger_output(test, &stdout, checks, "gdb")?;
    Ok(())
}

fn check_debugger_output(
    test: &TestCase,
    debugger_stdout: &str,
    checks: &[String],
    debugger_name: &str,
) -> Result<()> {
    let dbg_lines: Vec<&str> = debugger_stdout.lines().collect();
    let mut last_idx = 0;
    let mut missing: Vec<&str> = Vec::new();

    for pattern in checks {
        let pattern = pattern.trim();
        let fragments: Vec<&str> = pattern.split("[...]").filter(|f| !f.is_empty()).collect();
        if fragments.is_empty() {
            continue;
        }

        let mut found = false;
        for offset in last_idx..dbg_lines.len() {
            let line = dbg_lines[offset].trim();
            if check_single_debug_line(line, &fragments) {
                last_idx = offset + 1;
                found = true;
                break;
            }
        }
        if !found {
            missing.push(pattern);
        }
    }

    if !missing.is_empty() {
        let file = test.path.file_name().unwrap().to_string_lossy();
        let mut msg = format!(
            "{debugger_name} check directive(s) from `{file}` not found in debugger output:\n"
        );
        for pat in &missing {
            msg += &format!("  `{pat}`\n");
        }
        let preview: Vec<&str> = dbg_lines.iter().take(40).copied().collect();
        msg += &format!("--- debugger output (first {} lines) ---\n", preview.len());
        for l in &preview {
            msg += &format!("  {l}\n");
        }
        bail!(msg);
    }

    Ok(())
}

fn check_single_debug_line(line: &str, fragments: &[&str]) -> bool {
    let mut rest = line;
    for fragment in fragments {
        let Some(pos) = rest.find(fragment) else {
            return false;
        };
        rest = &rest[pos + fragment.len()..];
    }

    true
}
