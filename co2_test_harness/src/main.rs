use anyhow::Result;
use clap::Parser;

mod cli;
mod compiler;
mod error;
mod suite;
mod test_case;
mod ui;
mod util;

use cli::Cli;
use compiler::build_compilers;
use suite::{Stats, run_tests};
use util::workspace_root;

fn main() {
    if let Err(e) = run_main() {
        eprintln!("co2_test_harness: {e:#}");
        std::process::exit(1);
    }
}

fn run_main() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    build_compilers(&root, cli.coverage)?;

    let coverage_dir = if cli.coverage {
        let dir = root.join("target").join("co2-coverage");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Some(dir)
    } else {
        None
    };

    let mut stats = Stats::default();
    run_tests(
        &root,
        cli.filter.as_deref(),
        coverage_dir.as_deref(),
        &mut stats,
    )?;

    if let Some(dir) = coverage_dir {
        generate_coverage_report(&root, &dir)?;
    }

    eprintln!(
        "summary: passed={}, failed={}, skipped={}",
        stats.passed, stats.failed, stats.skipped
    );

    if stats.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn generate_coverage_report(root: &std::path::Path, dir: &std::path::Path) -> Result<()> {
    use std::process::Command;

    eprintln!("generating coverage report...");

    let merged_data = dir.join("merged.profdata");
    let multicall_bin = root.join("target").join("debug").join("co2-multicall");

    let mut cmd = Command::new("llvm-profdata");
    cmd.arg("merge").arg("-sparse");

    let mut found_any = false;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("profraw") {
                cmd.arg(entry.path());
                found_any = true;
            }
        }
    }

    if !found_any {
        eprintln!("no .profraw files found in {}", dir.display());
        return Ok(());
    }

    let status = cmd.arg("-o").arg(&merged_data).status();

    if let Err(e) = status {
        eprintln!("failed to run llvm-profdata: {e}. Ensure llvm-tools are installed.");
        return Ok(());
    }

    let report_dir = dir.join("report");
    let mut cov_cmd = Command::new("llvm-cov");
    cov_cmd
        .arg("show")
        .arg(&multicall_bin)
        .arg(format!("-instr-profile={}", merged_data.display()))
        .arg("-format=html")
        .arg(format!("-output-dir={}", report_dir.display()));

    // Only show coverage for our own crates
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if (name_str.starts_with("co2") || name_str == "rustc_public_generative")
                && entry.path().is_dir()
            {
                cov_cmd.arg(entry.path());
            }
        }
    }

    let status = cov_cmd.status();

    if let Ok(s) = status
        && s.success()
    {
        eprintln!("coverage report generated at: {}/index.html", report_dir.display());

        // Also show a summary report in the terminal
        let mut report_cmd = Command::new("llvm-cov");
        report_cmd
            .arg("report")
            .arg(&multicall_bin)
            .arg(format!("-instr-profile={}", merged_data.display()));

        // Use same filters as 'show'
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if (name_str.starts_with("co2") || name_str == "rustc_public_generative")
                    && entry.path().is_dir()
                {
                    report_cmd.arg(entry.path());
                }
            }
        }

        if let Ok(output) = report_cmd.output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(total_line) = stdout.lines().find(|l| l.starts_with("TOTAL")) {
                let parts: Vec<&str> = total_line.split_whitespace().collect();
                if parts.len() >= 10 {
                    let func_cov = parts[6];
                    let line_cov = parts[9];
                    eprintln!("Total Coverage: Functions={func_cov}, Lines={line_cov}");
                }
            }
        }
    } else {
        eprintln!("failed to generate coverage report with llvm-cov");
    }

    Ok(())
}
