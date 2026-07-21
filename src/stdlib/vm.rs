//! Node `vm` module — code compilation and evaluation reusing node-js's own
//! engine.
//!
//! Fidelity and honesty about scope: node-js runs on a single global heap with
//! one set of module-level globals (see `host::JsHost`). It has NO facility for a
//! second, isolated global object, so `vm` here provides genuine **evaluation**
//! but NOT the **context isolation** Node's `vm` is designed around:
//!
//! * `runInThisContext(code)` — REAL: compiles `code` and runs it on the current
//!   host through the exact `compile` → `load_merged` → `run_chunk_on` path the
//!   module loader and REPL use, returning the completion value (the value of the
//!   last expression). Code sees and mutates the current globals — which is
//!   precisely what `runInThisContext` is supposed to do.
//! * `runInNewContext(code[, sandbox])` — NOT ISOLATED (documented): there is no
//!   separate global object to create. As a pragmatic contextify emulation, the
//!   sandbox's own properties are merged into the shared global scope before the
//!   run and copied back into the sandbox object afterward, so the common
//!   `runInNewContext(src, sandbox)` read/write pattern works. It does NOT hide
//!   the surrounding globals and does NOT restore them — this is stated plainly,
//!   never claimed as isolation.
//! * `createContext(obj)` — no-op passthrough: returns `obj` (or a fresh object).
//!   node-js has no distinct context to contextify; this exists so
//!   `createContext` call sites don't throw.
//! * `isContext(obj)` — returns `true` for any object (everything shares the one
//!   context here).
//! * `Script` — a compiled-code holder (`@@native = "Script"`): `new Script(code)`
//!   stores the source; `.runInThisContext()` / `.runInNewContext([sandbox])` run
//!   it on demand via the same paths as the free functions.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
    "runInThisContext",
    "runInNewContext",
    "runInContext",
    "createContext",
    "createScript",
    "compileFunction",
    "isContext",
];

/// Methods dispatched on an `@@native = "Script"` object (reported to the parent
/// for `instance_has_method` wiring).
pub const SCRIPT_METHODS: &[&str] = &["runInThisContext", "runInNewContext"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "runInThisContext" => run_code(&super::arg_str(args, 0)),
        "runInNewContext" => run_in_context(&super::arg_str(args, 0), args.get(1)),
        // `runInContext(code, contextifiedObject[, options])` — node-js has one
        // shared context, so it behaves exactly like `runInNewContext`: the
        // context object's own props are merged into the global scope, the code
        // runs, and mutated keys are copied back (see `run_in_context`).
        "runInContext" => run_in_context(&super::arg_str(args, 0), args.get(1)),
        // `createScript` is the legacy factory form of `new vm.Script(code)`.
        "createScript" => construct(args),
        "compileFunction" => compile_function(args),
        // Contextify passthrough: return the sandbox (or a fresh object). node-js
        // has one global context, so there is nothing to isolate.
        "createContext" => Ok(match args.first() {
            Some(o) if with_host(|h| matches!(h.get(o), Some(JsObj::Object(_)))) => o.clone(),
            _ => with_host(|h| h.new_object(IndexMap::new())),
        }),
        // Everything shares the single global context.
        "isContext" => Ok(Value::Bool(with_host(|h| {
            matches!(
                h.get(args.first().unwrap_or(&Value::Undef)),
                Some(JsObj::Object(_))
            )
        }))),
        _ => return None,
    })
}

/// `new vm.Script(code)` → a Script object holding the source.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    let code = super::arg_str(args, 0);
    Ok(with_host(|h| {
        let code_val = h.new_str(code);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Script"));
        m.insert("@@code".into(), code_val);
        h.new_object(m)
    }))
}

/// Dispatch a method on a Script instance (`@@native = "Script"`).
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let code = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@code").map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    });
    match method {
        "runInThisContext" => run_code(&code),
        "runInNewContext" | "runInContext" => run_in_context(&code, args.first()),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Compile `code` and run it on the current host, returning the completion value
/// (the last expression's value). This is the same nested-run path the module
/// loader uses (`compile` → `load_merged` → `host::run_chunk_on`, cf.
/// `module.rs`), so it is re-entrant-safe when called from within a running
/// script.
fn run_code(code: &str) -> Result<Value, String> {
    // compile_completion leaves the final expression's value as the result.
    let prog = crate::compile_completion(code)?;
    let main = crate::load_merged(prog);
    crate::host::run_chunk_on(main)
}

/// `vm.compileFunction(code, params[, options])` — REAL: wrap `code` in a
/// function literal with the requested parameter names and run it, returning the
/// resulting callable (a genuine JS function value on the heap, invocable like any
/// other). This reuses the same compile→run path as the rest of the module.
///
/// Scope note: the `options.parsingContext` / `contextExtensions` isolation knobs
/// are ignored (node-js has one shared context, same limitation as
/// `runInNewContext`); the produced function closes over the shared globals.
fn compile_function(args: &[Value]) -> Result<Value, String> {
    let code = super::arg_str(args, 0);
    // `params` is an array of parameter-name strings (absent → no params).
    let params = match args.get(1) {
        Some(v) => with_host(|h| match h.get(v) {
            Some(JsObj::Array(items)) => items
                .iter()
                .map(|it| h.str_of(it))
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        }),
        None => String::new(),
    };
    let src = format!("(function anonymous({params}\n) {{\n{code}\n}})");
    run_code(&src)
}

/// `runInNewContext`/Script.runInNewContext: NOT isolated. Merge the sandbox's
/// own (non-hidden) properties into the shared global scope, run, then copy those
/// keys back into the sandbox object. Surrounding globals stay visible.
fn run_in_context(code: &str, sandbox: Option<&Value>) -> Result<Value, String> {
    let keys: Vec<(String, Value)> = match sandbox {
        Some(s) => with_host(|h| match h.get(s) {
            Some(JsObj::Object(p)) => p
                .iter()
                .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Vec::new(),
        }),
        None => Vec::new(),
    };
    with_host(|h| {
        for (k, v) in &keys {
            h.set_global(k, v.clone());
        }
    });
    let r = run_code(code);
    if let Some(s) = sandbox {
        for (k, _) in &keys {
            if let Some(nv) = with_host(|h| h.read_global(k)) {
                with_host(|h| {
                    if let Some(JsObj::Object(p)) = h.get_mut(s) {
                        p.insert(k.clone(), nv);
                    }
                });
            }
        }
    }
    r
}
