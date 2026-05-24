use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "co2_test_harness")]
pub struct Cli {
    /// Optional glob matched against the workspace-relative test path.
    pub filter: Option<String>,

    /// Run tests with code coverage instrumented.
    #[arg(long)]
    pub coverage: bool,
}
