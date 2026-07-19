//! The `node` binary entry point.
//!
//! Dispatch: `-e <src>` runs a one-liner; a positional `.js` file is executed;
//! otherwise stdin is read and run as a script. Errors go to stderr in terse
//! `node: <reason>` form; nothing else is printed.

use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = nodejs::cli::parse();

    if let Some(src) = cli.eval {
        return run_source(&src);
    }

    if let Some(file) = cli.file {
        return match nodejs::eval_file(&file) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        };
    }

    // No file and no -e: run stdin as a script.
    let src = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    run_source(&src)
}

fn run_source(src: &str) -> ExitCode {
    match nodejs::eval_str(src) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("node: {msg}");
    ExitCode::FAILURE
}
