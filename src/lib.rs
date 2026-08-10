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
pub mod tiers;
pub mod utf16;

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

/// `process.exitCode` as the program left it, or `None` if it was never set.
///
/// The binary reads this after a run completes to pick its own status — Node
/// exits with `process.exitCode` when the loop drains normally, so a script
/// that signals failure that way (rather than by throwing or calling
/// `process.exit`) is reported as a failure rather than as success.
pub fn exit_code() -> Option<i32> {
    host::with_host(|h| h.exit_code)
}

/// Run the `exit` event for a program that died on an uncaught exception, and
/// report the status to leave with.
///
/// Node fires `exit` on this path too, and an uncaught exception FORCES the
/// code to 1 — overriding any `process.exitCode` the script had already set —
/// while a code the handler itself assigns still wins. Verified on node
/// v26.7.0: `process.exitCode = 3; process.on('exit', c => console.log(c));
/// throw new Error('z')` prints `1` and exits 1, and
/// `process.on('exit', () => { process.exitCode = 9 }); throw new Error('z')`
/// exits 9.
pub fn exit_code_after_failure() -> i32 {
    host::with_host(|h| h.exit_code = Some(1));
    let _ = stdlib::process::emit_exit_event(1);
    host::with_host(|h| h.exit_code).unwrap_or(1)
}

/// Compile `src` and run it on the LIVE host — no reset, no event-loop drain —
/// in the GLOBAL scope, returning its completion value.
///
/// This is the ONE runtime-source evaluator on this frontend. Every construct
/// that turns a source string into a running program funnels through here:
/// the CommonJS module wrapper (`module::compile_wrapper`), `vm.runInThisContext`
/// / `vm.Script` / `vm.compileFunction`, `new Function` / `Function(...)`
/// (`builtins::dynamic_function`), and the internal JS factories
/// (`util.promisify`, `stream/promises`, `stream/consumers`,
/// `performance.timerify`, `module.builtinModules`). Each of those used to carry
/// its own `compile_completion` → `load_merged` → `run_chunk_on` triple — seven
/// copies of the same three lines — and every one of them inherited the same
/// bug: `run_chunk_on` executes on whatever frame is CURRENT, so nested source
/// saw the calling function's locals. Measured against node v26.7.0,
/// `function outer(){ let secret = 1; return require('./m.js'); }` with `m.js` =
/// `module.exports = typeof secret` is `"undefined"` there and was `"number"`
/// here; `vm.runInThisContext('typeof loc')` likewise. `run_chunk_in_global_scope`
/// fixes it once, for all of them.
pub fn eval_in_global_scope(src: &str) -> Result<Value, String> {
    let prog = compile_completion(src)?;
    let chunk = load_merged(prog);
    host::run_chunk_in_global_scope(chunk)
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
///
/// This is the `node -e` entry point; [`eval_str_from`] names the other
/// source-on-the-command-line one, which reports a different `__filename`.
pub fn eval_str(src: &str) -> Result<Value, String> {
    eval_str_from(src, "[eval]")
}

/// [`eval_str`] with the entry-point NAME node reports for it: `[eval]` for
/// `-e`, `[stdin]` for source piped in. The two are observably different —
/// `__filename`, `module.id` and a stack frame's file all carry it.
pub fn eval_str_from(src: &str, origin: &str) -> Result<Value, String> {
    host::reset_host();
    // `node -e` resolves top-level `require` from the current working directory.
    if let Ok(cwd) = std::env::current_dir() {
        module::set_entry_dir(cwd);
    }
    module::install_entry_globals(origin);
    run_compiled(compile_or_load(src)?)
}

/// `node -p <src>`: evaluate as `-e` does, then write the program's COMPLETION
/// value through the `console.log` formatter, exactly as Node's
/// `--print` does (`node -p '[1,2]'` prints `[ 1, 2 ]`, `node -p '"s"'` prints
/// the bare `s`). Side effects still happen, so `node -p 'console.log("x")'`
/// prints `x` and then `undefined`.
///
/// Deliberately compiled with [`compile_completion`] rather than through the
/// source-keyed rkyv cache: the cache is keyed by source TEXT alone, so a
/// `-p`-shaped chunk and an `-e`-shaped chunk for the same string would alias.
pub fn eval_str_print(src: &str, origin: &str) -> Result<(), String> {
    host::reset_host();
    if let Ok(cwd) = std::env::current_dir() {
        module::set_entry_dir(cwd);
    }
    module::install_entry_globals(origin);
    let value = run_compiled(compile_completion(src)?)?;
    let line = stdlib::util::format(std::slice::from_ref(&value));
    host::with_host(|h| h.write_out(&format!("{line}\n"), false));
    Ok(())
}

/// Run a JS source string on a fresh host with `globals` bound and the
/// program's output captured in-process, returning the program's outcome
/// alongside everything it wrote.
///
/// This is the entry point for an embedder rather than for the `node` binary,
/// and it exists because [`eval_str`] cannot serve one: it resets the host
/// first, which wipes any global installed beforehand, and it lets
/// `console.log` reach the real stdout, which corrupts a host that owns the
/// terminal. Both are fixed here — the globals are seeded *after* the reset,
/// and every write the program makes lands in the returned string.
///
/// The outcome and the output are returned separately (rather than the output
/// only on success) because a program that prints and *then* throws produced
/// both, and an embedder generally wants to show both.
///
/// Globals are given as text and interned as real JS strings here. They are
/// deliberately *not* `Value`: strings live on this host's heap as
/// `JsObj::Str`, so a `Value::Str` a caller builds is at best coerced and at
/// worst method-less. Handing the host text and letting it intern removes that
/// trap, and matches the sibling runtimes' embedder entry points.
///
/// ```no_run
/// let (result, out) = nodejs::eval_str_captured("console.log(stdin.toUpperCase())", &[("stdin", "hi")]);
/// assert!(result.is_ok());
/// assert_eq!(out, "HI\n");
/// ```
pub fn eval_str_captured(src: &str, globals: &[(&str, &str)]) -> (Result<Value, String>, String) {
    host::reset_host();
    if let Ok(cwd) = std::env::current_dir() {
        module::set_entry_dir(cwd);
    }
    host::with_host(|h| {
        for (name, text) in globals {
            let value = h.new_str(*text);
            h.set_global(name, value);
        }
        h.begin_capture();
    });
    let result = compile_or_load(src).and_then(run_compiled);
    let output = host::with_host(|h| h.end_capture());
    (result, output)
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
    // `__filename` is the entry script's REALPATH, not the path that was typed:
    // Node's loader calls `toRealPath` on the main module, so a script reached
    // through a symlinked directory reports the link TARGET. (`process.argv[1]`
    // is the opposite — it keeps the spelling; both measured on node v26.7.0.)
    let entry = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| stdlib::path::resolve_one(path));
    module::install_entry_globals(&entry);
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
