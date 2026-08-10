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

    /// Start the interactive REPL.
    #[arg(long = "repl")]
    pub repl: bool,

    /// Speak the Language Server Protocol over stdio.
    #[arg(long = "lsp")]
    pub lsp: bool,

    /// Speak the Debug Adapter Protocol over stdio.
    #[arg(long = "dap")]
    pub dap: bool,

    /// Ahead-of-time compile the script to a standalone native executable.
    #[arg(long = "build")]
    pub build: bool,

    /// Print the compiled fusevm bytecode for the script and exit.
    #[arg(long = "dump-bytecode")]
    pub dump_bytecode: bool,

    /// Print the lexer token stream for the script and exit.
    #[arg(long = "dump-tokens")]
    pub dump_tokens: bool,

    /// Print the parsed AST for the script and exit.
    #[arg(long = "dump-ast")]
    pub dump_ast: bool,

    /// Print a fusevm bytecode disassembly listing for the script and exit.
    #[arg(long = "disasm")]
    pub disasm: bool,

    /// Run the script, then report which fusevm tiers took each of its chunks.
    #[arg(long = "tiers")]
    pub tiers: bool,

    /// Silence all `process.emitWarning` output (Node's `--no-warnings`).
    #[arg(long = "no-warnings")]
    pub no_warnings: bool,

    /// Silence DeprecationWarnings only (Node's `--no-deprecation`).
    #[arg(long = "no-deprecation")]
    pub no_deprecation: bool,

    /// Suppress the one-time "(Use `node --trace-warnings ...`)" hint on warnings.
    #[arg(long = "trace-warnings")]
    pub trace_warnings: bool,

    /// Suppress that hint for DeprecationWarnings (Node's `--trace-deprecation`).
    #[arg(long = "trace-deprecation")]
    pub trace_deprecation: bool,

    /// The `.js` script to run (omit with --repl / --lsp / --dap / -e).
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

/// How Node divides the command line, which is NOT the raw `argv` the OS handed
/// over: the flags the RUNTIME consumed go to `process.execArgv`, and
/// `process.argv` is `[execPath, entryScript, ...userArgs]` with those flags
/// removed. The split is entry-point dependent and observably so —
/// `node -e 'src' z` reports `execArgv = ["-e","src"]` and `argv = [exec,"z"]`,
/// with NO `argv[1]`, while `node s.js z` reports `execArgv = []` and
/// `argv = [exec, "/abs/s.js", "z"]`. Measured on node v26.7.0.
pub struct Argv {
    /// Runtime flags, including `-e`/`--eval` and its source.
    pub exec: Vec<String>,
    /// The entry script as given, or `None` for `-e` and for stdin.
    pub script: Option<String>,
    /// Everything after the entry point, passed through to the program.
    pub user: Vec<String>,
}

/// Compute [`Argv`] from the raw command line.
///
/// This walks the raw arguments rather than reading the parsed [`Cli`], because
/// clap assigns the FIRST positional to `file` even under `-e`, where that
/// positional is really the program's own first argument.
pub fn split_argv<I: IntoIterator<Item = String>>(raw: I) -> Argv {
    let mut out = Argv {
        exec: Vec::new(),
        script: None,
        user: Vec::new(),
    };
    let mut it = raw.into_iter().skip(1);
    let mut eval_seen = false;
    while let Some(a) = it.next() {
        if a == "-e" || a == "--eval" {
            out.exec.push(a);
            if let Some(src) = it.next() {
                out.exec.push(src);
            }
            eval_seen = true;
        } else if a.starts_with('-') && a != "-" {
            // `-` is Node's "read stdin" entry point, not a flag.
            out.exec.push(a);
        } else {
            // The first non-flag ends the runtime's own arguments. Under `-e`
            // there is no entry script, so it is already a program argument.
            if !eval_seen {
                out.script = Some(a);
            } else {
                out.user.push(a);
            }
            out.user.extend(it);
            break;
        }
    }
    out
}
