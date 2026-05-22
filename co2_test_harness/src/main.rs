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
    build_compilers(&root)?;

    let mut stats = Stats::default();
    run_tests(&root, cli.filter.as_deref(), &mut stats)?;

    eprintln!(
        "summary: passed={}, failed={}, skipped={}",
        stats.passed, stats.failed, stats.skipped
    );

    if stats.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
