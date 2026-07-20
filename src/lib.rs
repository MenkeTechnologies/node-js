//! node-js — JavaScript as a fusevm frontend.
//!
//! Pipeline: `lexer` → `parser` builds a JS AST → `compiler` lowers it to a
//! `fusevm::Chunk` (plus a table of function/arrow sub-chunks and try-block
//! chunks) → fusevm executes it, calling back into the `host` (through
//! registered builtins and the strict numeric hook) for every JS-specific
//! operation. There is no bespoke VM or JIT here — execution and codegen live in
//! fusevm.

pub mod aot;
pub mod aot_native;
pub mod ast;
pub mod banner;
pub mod builtins;
pub mod cache;
pub mod cli;
pub mod compiler;
pub mod dap;
pub mod host;
pub mod lexer;
pub mod lsp;
pub mod module;
pub mod parser;
pub mod regexp;
pub mod repl;
pub mod rust_ffi;
pub mod stdlib;

pub use fusevm::Value;

/// Compile a source string to a runnable program.
pub fn compile(src: &str) -> Result<compiler::Program, String> {
    let stmts = parser::parse(src)?;
    compiler::compile(&stmts, false)
}

/// Compile leaving the final top-level expression as the program's completion
/// value (for `vm.runInThisContext` / `eval`).
pub fn compile_completion(src: &str) -> Result<compiler::Program, String> {
    let stmts = parser::parse(src)?;
    compiler::compile_completion(&stmts, false)
}

/// Compile with per-statement DAP line markers enabled (`node --dap`).
pub fn compile_debug(src: &str) -> Result<compiler::Program, String> {
    let stmts = parser::parse(src)?;
    compiler::compile(&stmts, true)
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

/// Transparent bytecode cache: return the cached compiled `Program` for `src`
/// (skipping lex/parse/lower entirely), else compile it, store it in the
/// `~/.node-js/scripts.rkyv` shard, and return it. This runs on EVERY ordinary
/// `node foo.js` / `node -e` invocation, so scripts are rkyv-cached automatically
/// — not only under `--build`. Set `NODE_JS_TRACE=1` to log hit/miss to stderr
/// (silent otherwise; normal runs print nothing).
pub fn compile_or_load(src: &str) -> Result<compiler::Program, String> {
    if let Some(prog) = cache::load(src) {
        if std::env::var_os("NODE_JS_TRACE").is_some() {
            eprintln!(
                "node-js: cache HIT ({} ops, {} functions) — skipped lex/parse/lower",
                prog.main.ops.len(),
                prog.functions.len()
            );
        }
        return Ok(prog);
    }
    let prog = compile(src)?;
    let _ = cache::store(src, &prog);
    if std::env::var_os("NODE_JS_TRACE").is_some() {
        eprintln!(
            "node-js: cache MISS — compiled + stored ({} ops, {} functions)",
            prog.main.ops.len(),
            prog.functions.len()
        );
    }
    Ok(prog)
}

/// Parse/load, compile, and run a JS source string on a fresh host (rkyv-cached).
pub fn eval_str(src: &str) -> Result<Value, String> {
    host::reset_host();
    // `node -e` resolves top-level `require` from the current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        module::set_entry_dir(cwd);
    }
    run_compiled(compile_or_load(src)?)
}

/// Read and run a `.js` file (transparently rkyv-cached — see `compile_or_load`).
pub fn eval_file(path: &str) -> Result<Value, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    host::reset_host();
    // Top-level `require` in `node app.js` resolves from the entry file's dir.
    let dir = std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    module::set_entry_dir(dir);
    run_compiled(compile_or_load(&src)?)
}

/// Read and run a `.js` file under the DAP debugger.
pub fn eval_file_debug(path: &str) -> Result<Value, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let prog = compile_debug(&src)?;
    host::reset_host();
    host::set_debug_mode(true);
    let r = run_compiled(prog);
    host::set_debug_mode(false);
    r
}
