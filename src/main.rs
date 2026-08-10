//! The `node` binary entry point.
//!
//! Dispatch: `--lsp`/`--dap` speak their protocols over stdio; `--repl` (or no
//! file on a TTY) starts the interactive loop; `--build` AOT-compiles to a
//! standalone native executable; `--dump-bytecode` prints the lowered fusevm
//! chunk; `-e <src>` runs a one-liner; a positional `.js` file is executed;
//! otherwise stdin is read and run as a script. Errors go to stderr in terse
//! `node: <reason>` form; nothing else is printed.

use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = nodejs::cli::parse();
    nodejs::stdlib::process::install_argv();

    if cli.lsp {
        return match nodejs::lsp::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        };
    }
    if cli.dap {
        return match nodejs::dap::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        };
    }

    // `-p`/`--print` is `-e` plus a print of the completion value. When both are
    // given Node keeps the LAST one on the command line; clap does not preserve
    // that order, so the raw argv decides.
    match (cli.eval, cli.print) {
        (Some(e), Some(p)) => {
            return if last_eval_flag_is_print() {
                run_print(&p)
            } else {
                run_source(&e)
            }
        }
        (Some(e), None) => return run_source(&e),
        (None, Some(p)) => return run_print(&p),
        (None, None) => {}
    }

    // `node -` is Node's EXPLICIT stdin entry point: `-` is the entry argument,
    // not a filename, and everything after it is a program argument. Treating it
    // as a path made `echo … | node - q` fail with `cannot read -`.
    if cli.file.as_deref() == Some("-") {
        return run_stdin();
    }

    if let Some(file) = cli.file {
        if cli.dump_bytecode {
            return match dump(&file) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            };
        }
        if cli.dump_tokens {
            return finish(dump_tokens(&file));
        }
        if cli.dump_ast {
            return finish(dump_ast(&file));
        }
        if cli.disasm {
            return finish(disasm(&file));
        }
        if cli.tiers {
            return finish(tiers(&file));
        }
        if cli.build {
            return match nodejs::aot::build(&file) {
                Ok(msg) => {
                    // A build report is explicit user-requested output.
                    println!("{msg}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            };
        }
        return match nodejs::eval_file(&file) {
            Ok(_) => program_status(),
            Err(e) => fail_program(&e),
        };
    }

    if cli.repl || atty_stdin() {
        nodejs::repl::run();
        return ExitCode::SUCCESS;
    }

    // No file and non-interactive stdin: run stdin as a script.
    run_stdin()
}

/// Read the whole of stdin and run it as a script. Node names this entry point
/// `[stdin]` (not `[eval]`) in `__filename`, `module.id` and stack frames.
fn run_stdin() -> ExitCode {
    let src = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    match nodejs::eval_str_from(&src, "[stdin]") {
        Ok(_) => program_status(),
        Err(e) => fail_program(&e),
    }
}

fn run_source(src: &str) -> ExitCode {
    match nodejs::eval_str(src) {
        Ok(_) => program_status(),
        Err(e) => fail_program(&e),
    }
}

fn run_print(src: &str) -> ExitCode {
    match nodejs::eval_str_print(src, "[eval]") {
        Ok(()) => program_status(),
        Err(e) => fail_program(&e),
    }
}

/// Whether the last of the `-e`/`-p` family on the command line was a print
/// form. `node -e 'console.log(9)' -p '1+1'` prints only `2` on node v26.7.0.
fn last_eval_flag_is_print() -> bool {
    std::env::args()
        .rfind(|a| matches!(a.as_str(), "-e" | "--eval" | "-p" | "--print"))
        .map(|a| a == "-p" || a == "--print")
        .unwrap_or(false)
}

fn dump(file: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let prog = nodejs::compile(&src)?;
    println!("== main ==\n{:#?}", prog.main.ops);
    for (name, f) in &prog.functions {
        let params: Vec<&str> = f.params.iter().map(|p| p.name.as_str()).collect();
        println!(
            "== function {name} ({}) ==\n{:#?}",
            params.join(", "),
            f.chunk.ops
        );
    }
    for (i, t) in prog.tries.iter().enumerate() {
        println!("== try #{i} ==\n{:#?}", t.block.ops);
    }
    Ok(())
}

/// `--dump-tokens`: print the lexer token stream, one `line\tTok` per line.
fn dump_tokens(file: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    for t in nodejs::lexer::lex(&src)? {
        println!("{}\t{:?}", t.line, t.tok);
    }
    Ok(())
}

/// `--dump-ast`: print the parsed JS AST.
fn dump_ast(file: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let stmts = nodejs::parser::parse(&src)?;
    println!("{stmts:#?}");
    Ok(())
}

/// `--disasm`: print a fusevm bytecode disassembly of the main chunk, every
/// compiled function, and every try block, via the shared
/// `fusevm::Chunk::disassemble` (distinct from `--dump-bytecode`'s raw `.ops`).
fn disasm(file: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let prog = nodejs::compile(&src)?;
    println!("; node fusevm — main\n{}", prog.main.disassemble());
    for (name, f) in &prog.functions {
        let params: Vec<&str> = f.params.iter().map(|p| p.name.as_str()).collect();
        println!(
            "; node fusevm — function {name} ({})\n{}",
            params.join(", "),
            f.chunk.disassemble()
        );
    }
    for (i, t) in prog.tries.iter().enumerate() {
        println!("; node fusevm — try #{i}\n{}", t.block.disassemble());
    }
    Ok(())
}

/// `--tiers`: run the script, then report which fusevm execution tier took
/// each of its chunks — asked of fusevms own eligibility and cache
/// predicates, so the answer comes from the compiler that would have done the
/// work. The programs own output precedes the report.
fn tiers(file: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    println!("{}", nodejs::tiers::report(&src)?);
    Ok(())
}

fn finish(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn atty_stdin() -> bool {
    // SAFETY: isatty is a pure query on the stdin fd.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("node: {msg}");
    ExitCode::FAILURE
}

/// A PROGRAM that died on an uncaught exception, as opposed to the runtime
/// failing to start one. Node still runs the `exit` listeners on this path, and
/// a code one of them assigns is the one the process leaves with.
fn fail_program(msg: &str) -> ExitCode {
    let code = nodejs::exit_code_after_failure();
    eprintln!("node: {msg}");
    ExitCode::from((code & 0xff) as u8)
}

/// The status a program that ran to completion leaves behind: `process.exitCode`
/// if the script (or an `exit` listener) set one, else 0.
///
/// This is the whole point of `process.exitCode` and it was not wired: a script
/// whose only failure signal is `process.exitCode = 1` — the shape every test
/// runner and lint wrapper uses, because it lets the loop drain first — exited
/// 0, reporting success. Measured on node v26.7.0, `node -e 'process.exitCode
/// = 3'` exits 3.
fn program_status() -> ExitCode {
    match nodejs::exit_code() {
        Some(c) => ExitCode::from((c & 0xff) as u8),
        None => ExitCode::SUCCESS,
    }
}
