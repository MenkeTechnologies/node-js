//! Command-line interface for the `node` binary.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "node",
    version,
    about = "JavaScript on fusevm — a compiled JS runtime (bytecode VM + Cranelift JIT)",
    long_about = None,
)]
pub struct Cli {
    /// Evaluate a one-liner instead of a file (`node -e 'console.log(1+1)'`).
    #[arg(short = 'e', long = "eval", value_name = "SRC")]
    pub eval: Option<String>,

    /// The `.js` script to run (omit with -e or to read stdin).
    #[arg(value_name = "FILE")]
    pub file: Option<String>,

    /// Arguments passed through to the JS program.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub argv: Vec<String>,
}

/// Parse the process arguments.
pub fn parse() -> Cli {
    Cli::parse()
}
