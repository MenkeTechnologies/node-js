//! node-js — JavaScript as a fusevm frontend.
//!
//! Pipeline: `lexer` → `parser` builds a JS AST → `compiler` lowers it to a
//! `fusevm::Chunk` (plus a table of function/arrow sub-chunks and try-block
//! chunks) → fusevm executes it, calling back into the `host` (through
//! registered builtins and the strict numeric hook) for every JS-specific
//! operation. There is no bespoke VM or JIT here — execution and codegen live in
//! fusevm.

pub mod ast;
pub mod builtins;
pub mod cli;
pub mod compiler;
pub mod host;
pub mod lexer;
pub mod parser;

pub use fusevm::Value;

/// Compile a source string to a runnable program.
pub fn compile(src: &str) -> Result<compiler::Program, String> {
    let stmts = parser::parse(src)?;
    compiler::compile(&stmts)
}

/// Rebase a freshly compiled program's func/try ids above those already loaded
/// on the host, install its functions/tries, and return the (rebased) main
/// chunk to run.
pub fn load_merged(mut prog: compiler::Program) -> fusevm::Chunk {
    let (func_off, try_off) = host::with_host(|h| h.program_offsets());
    compiler::rebase_program(&mut prog, func_off, try_off);
    let compiler::Program {
        main,
        functions,
        tries,
    } = prog;
    let funcs: Vec<host::FuncDef> = functions.into_iter().map(|(_, f)| f).collect();
    host::with_host(|h| h.load_program(funcs, tries));
    main
}

/// Run an already-compiled program on the current host.
pub fn run_compiled(prog: compiler::Program) -> Result<Value, String> {
    host::run_main(load_merged(prog))
}

/// Parse/compile and run a JS source string on a fresh host.
pub fn eval_str(src: &str) -> Result<Value, String> {
    host::reset_host();
    run_compiled(compile(src)?)
}

/// Read and run a `.js` file.
pub fn eval_file(path: &str) -> Result<Value, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    host::reset_host();
    run_compiled(compile(&src)?)
}
