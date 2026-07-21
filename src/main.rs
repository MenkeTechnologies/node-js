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

    if let Some(src) = cli.eval {
        return run_source(&src);
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
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        };
    }

    if cli.repl || atty_stdin() {
        nodejs::repl::run();
        return ExitCode::SUCCESS;
    }

    // No file and non-interactive stdin: run stdin as a script.
    let src = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    run_source(&src)
}

fn run_source(src: &str) -> ExitCode {
    match nodejs::eval_str(src) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
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
