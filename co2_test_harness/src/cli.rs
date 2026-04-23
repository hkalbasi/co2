use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "co2_test_harness")]
pub struct Cli {
    #[arg(value_enum, default_value_t = SuiteArg::All)]
    pub suite: SuiteArg,
    #[arg(long)]
    pub filter: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SuiteArg {
    All,
    Ui,
    Run,
    Debuginfo,
}
