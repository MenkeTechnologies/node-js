//! CommonJS module loader.
//!
//! Node's `require()` semantics, layered on the existing engine — no bespoke VM
//! primitive. A `.js` file is wrapped in the canonical Node module wrapper
//! `(function (exports, require, module, __dirname, __filename) { … })`, compiled
//! through the ordinary `compile` → `load_merged` path to obtain the wrapper
//! FUNCTION value, then `host::invoke`d with a fresh `module = { exports: {} }`.
//! Whatever the body assigns to `module.exports` (or hangs off `exports`) is the
//! module's value; it is cached by resolved absolute path so a second `require`
//! of the same file returns the identical object and circular requires observe
//! the partially-filled `exports`.
//!
//! Core modules (`fs`, `path`, `http`, …) short-circuit to their native
//! `JsObj::Builtin` namespace (see `stdlib::resolve`) and are never read from
//! disk. Everything else — relative paths, JSON files, and bare `node_modules`
//! packages with their `package.json` `"exports"`/`"main"` and `index.js`
//! fallbacks — resolves on the real filesystem and runs the genuine, unmodified
//! source.
//!
//! Per-module `require` is a real JS closure that bakes in the defining module's
//! directory, so a `require(...)` deferred inside a function called much later
//! still resolves against the module that defined it (a single global
//! "current dir" would resolve against the wrong module). The closure is minted
//! by a one-time compiled factory (`FACTORY`) invoked with the directory string;
//! it dispatches back into this loader through the `__cjs_require` /
//! `__cjs_resolve` global native builtins.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::host::{self, with_host, JsObj};
use fusevm::Value;

thread_local! {
    /// Require cache: resolved absolute path → the `module` object (its `.exports`
    /// is re-read on every hit, matching Node — `module.exports = X` reassignment
    /// is observed by later requires).
    static CACHE: RefCell<HashMap<PathBuf, Value>> = RefCell::new(HashMap::new());
    /// Base directory the ENTRY script's top-level `require` resolves against
    /// (the dir of `node app.js`, or cwd for `node -e`).
    static ENTRY_DIR: RefCell<PathBuf> = RefCell::new(std::env::current_dir().unwrap_or_default());
    /// The compiled per-module `require`-closure factory (see module docs),
    /// minted once per host and reused for every module.
    static FACTORY: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// The compiled synthetic-CallSite-array factory (for `Error.captureStackTrace`
    /// under a custom `Error.prepareStackTrace`), minted once per host.
    static CALLSITE_FACTORY: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// Monotonic counter feeding the temporary capture-variable name used to read
    /// a compiled expression's value out of the shared module scope.
    static SEQ: RefCell<u64> = const { RefCell::new(0) };
}

/// Clear all per-host loader state. Called from `host::reset_host` so a fresh
/// eval (which rebuilds the heap) never reuses a stale heap handle.
pub fn reset() {
    CACHE.with(|c| c.borrow_mut().clear());
    FACTORY.with(|f| *f.borrow_mut() = None);
    CALLSITE_FACTORY.with(|f| *f.borrow_mut() = None);
    SEQ.with(|s| *s.borrow_mut() = 0);
    ENTRY_DIR.with(|d| *d.borrow_mut() = std::env::current_dir().unwrap_or_default());
}

/// Set the base directory the ENTRY script's `require` resolves against.
pub fn set_entry_dir(dir: PathBuf) {
    ENTRY_DIR.with(|d| *d.borrow_mut() = dir);
}

/// The ENTRY script's base directory.
pub fn entry_dir() -> PathBuf {
    ENTRY_DIR.with(|d| d.borrow().clone())
}

// ── resolution ───────────────────────────────────────────────────────────────

/// Append `.ext` to a path (Node appends the extension, it does not replace an
/// existing one — `foo.min` → `foo.min.js`, not `foo.js`).
fn add_ext(p: &Path, ext: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// `require`-as-a-file: `p`, then `p.js`, then `p.json`. `.node` native addons
/// are skipped (unsupported), matching the resolution order minus that step.
fn load_as_file(p: &Path) -> Option<PathBuf> {
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    for ext in ["js", "json"] {
        let cand = add_ext(p, ext);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// `require`-as-a-directory: honor `package.json` `"exports"`/`"main"`, else
/// `index.js` / `index.json`.
fn load_as_dir(p: &Path) -> Option<PathBuf> {
    let pkg = p.join("package.json");
    if pkg.is_file() {
        if let Some(main) = pkg_entry(&pkg) {
            let mp = p.join(&main);
            if let Some(f) = load_as_file(&mp).or_else(|| load_index(&mp)) {
                return Some(f);
            }
        }
    }
    load_index(p)
}

/// `index.js` / `index.json` inside directory `p`.
fn load_index(p: &Path) -> Option<PathBuf> {
    for name in ["index.js", "index.json"] {
        let cand = p.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// The relative entry path a `package.json` declares: the `"."` (or main-string)
/// `"exports"` target if present, else `"main"`. Only the common `"exports"`
/// shapes are handled — a bare string, or an object whose `"."` maps to a string
/// or to `{ "require"/"default"/"node": "…" }`. Anything more exotic falls back
/// to `"main"`, then to the directory's `index.js`.
fn pkg_entry(pkg: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pkg).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(e) = exports_main(json.get("exports")) {
        return Some(strip_dot_slash(&e));
    }
    json.get("main")
        .and_then(|m| m.as_str())
        .map(strip_dot_slash)
}

/// Resolve the `"exports"` field down to a single relative path for the `"."`
/// (package root) entry, across the shapes CommonJS packages commonly ship.
fn exports_main(exports: Option<&serde_json::Value>) -> Option<String> {
    let exports = exports?;
    // `"exports": "./index.js"` — a bare string is the `"."` target.
    if let Some(s) = exports.as_str() {
        return Some(s.to_string());
    }
    let obj = exports.as_object()?;
    // Either a subpath map keyed by `"."`, or a bare conditions map at the root.
    let target = obj.get(".").unwrap_or(exports);
    condition_target(target)
}

/// Reduce an `"exports"` target — a string, or a conditions object — to a path,
/// preferring the CommonJS-relevant conditions (`require`/`node`/`default`).
fn condition_target(target: &serde_json::Value) -> Option<String> {
    if let Some(s) = target.as_str() {
        return Some(s.to_string());
    }
    let obj = target.as_object()?;
    for cond in ["require", "node", "default"] {
        if let Some(v) = obj.get(cond) {
            if let Some(s) = condition_target(v) {
                return Some(s);
            }
        }
    }
    None
}

/// Drop a leading `./` from a package-relative path.
fn strip_dot_slash(s: &str) -> String {
    s.strip_prefix("./").unwrap_or(s).to_string()
}

/// Resolve `spec` (already known to be a bare specifier) by walking parent
/// directories from `from_dir`, checking `<dir>/node_modules/<spec>` at each
/// level with the file-then-directory rules.
fn resolve_bare(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    let mut dir = Some(from_dir);
    while let Some(d) = dir {
        // Skip a `node_modules/node_modules` descent.
        if d.file_name().is_some_and(|n| n == "node_modules") {
            dir = d.parent();
            continue;
        }
        let candidate = d.join("node_modules").join(spec);
        if let Some(f) = load_as_file(&candidate).or_else(|| load_as_dir(&candidate)) {
            return Some(f);
        }
        dir = d.parent();
    }
    None
}

/// Resolve `spec` relative to `from_dir` to an absolute file path, or `None` if
/// no file matches (core modules are handled earlier, by the caller).
pub fn resolve(spec: &str, from_dir: &Path) -> Option<PathBuf> {
    let is_relative =
        spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == "..";
    let is_absolute = spec.starts_with('/');
    if is_relative || is_absolute {
        let base = if is_absolute {
            PathBuf::from(spec)
        } else {
            from_dir.join(spec)
        };
        return load_as_file(&base).or_else(|| load_as_dir(&base));
    }
    resolve_bare(spec, from_dir)
}

// ── loading / execution ──────────────────────────────────────────────────────

/// `require(spec)` from `from_dir`: the single entry point shared by the
/// top-level `require` builtin and the per-module `__cjs_require`. Returns the
/// module's exports value.
pub fn require(spec: &str, from_dir: &Path) -> Result<Value, String> {
    // Core module: the native namespace value, never a file (mirrors the legacy
    // `require` path — `require('events')` yields the EventEmitter ctor, etc.).
    if let Some(ns) = crate::stdlib::resolve(spec) {
        return Ok(with_host(|h| h.alloc(JsObj::Builtin(ns.to_string()))));
    }
    let path =
        resolve(spec, from_dir).ok_or_else(|| format!("Error: Cannot find module '{spec}'"))?;
    // A canonical absolute key so the same file required via different relative
    // specifiers shares one cache entry.
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    load_file(&path)
}

/// Load the resolved absolute file `path` (`.json` parses to its value; `.js`
/// runs through the module wrapper) and return its exports, caching by path.
fn load_file(path: &Path) -> Result<Value, String> {
    if let Some(cached) = CACHE.with(|c| c.borrow().get(path).cloned()) {
        // Re-read `.exports` — a cached module may have reassigned it.
        return Ok(module_exports(&cached));
    }
    if path.extension().is_some_and(|e| e == "json") {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let src = with_host(|h| h.new_str(text));
        let val = crate::builtins::call_builtin_function("JSON.parse", vec![src])?;
        // A JSON module's value IS the parsed data; cache a synthetic wrapper so
        // repeated requires share it.
        let module = new_module(val.clone());
        CACHE.with(|c| c.borrow_mut().insert(path.to_path_buf(), module));
        return Ok(val);
    }

    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

    // Compile the Node module wrapper to obtain the wrapper FUNCTION value.
    // A compile error is annotated with the offending file (Node does likewise).
    let wrapper = compile_wrapper(&source)
        .map_err(|e| format!("{e}\n    while loading {}", path.display()))?;

    // `module = { exports: {} }`, plus the aliases the wrapper receives.
    let exports = with_host(|h| h.new_object(indexmap::IndexMap::new()));
    let module = new_module(exports.clone());
    let require_fn = make_require(&dir)?;
    let (dirname, filename) = with_host(|h| {
        (
            h.new_str(dir.to_string_lossy().to_string()),
            h.new_str(path.to_string_lossy().to_string()),
        )
    });

    // Cache BEFORE running so a circular `require` back to this module observes
    // the partial `exports`.
    CACHE.with(|c| c.borrow_mut().insert(path.to_path_buf(), module.clone()));

    host::invoke(
        &wrapper,
        vec![exports, require_fn, module.clone(), dirname, filename],
        None,
    )?;

    Ok(module_exports(&module))
}

/// A fresh `module` object holding `exports`.
fn new_module(exports: Value) -> Value {
    with_host(|h| {
        let mut props = indexmap::IndexMap::new();
        props.insert("exports".to_string(), exports);
        h.new_object(props)
    })
}

/// Read `module.exports` (falls back to `undefined` for a malformed module).
fn module_exports(module: &Value) -> Value {
    with_host(|h| match h.get(module) {
        Some(JsObj::Object(p)) => p.get("exports").cloned().unwrap_or(Value::Undef),
        _ => Value::Undef,
    })
}

/// Compile `<source>` wrapped in the Node module wrapper and return the wrapper
/// FUNCTION value. Uses a fresh unique capture variable so a nested load in
/// progress cannot clobber the value before it is read.
fn compile_wrapper(source: &str) -> Result<Value, String> {
    let n = SEQ.with(|s| {
        let mut b = s.borrow_mut();
        *b += 1;
        *b
    });
    let var = format!("__cjs_w{n}");
    // A trailing newline before `})` guards a source ending in a `//` comment.
    let wrapped = format!(
        "var {var} = (function (exports, require, module, __dirname, __filename) {{\n{source}\n}});"
    );
    eval_binding(&wrapped, &var)
}

/// Compile+run a `var <name> = <expr>;` statement (expression statements pop
/// their value, so a binding is how we capture an expression's result out of the
/// shared module scope) and return the bound value. Runs on the LIVE host — no
/// reset, no event-loop drain.
fn eval_binding(src: &str, name: &str) -> Result<Value, String> {
    let prog = crate::compile(src)?;
    let main = crate::load_merged(prog);
    host::run_chunk_on(main)?;
    with_host(|h| h.read_name(name))
        .ok_or_else(|| format!("module loader: failed to capture '{name}'"))
}

/// Build a per-module `require` closure bound to `dir` (see module docs).
fn make_require(dir: &Path) -> Result<Value, String> {
    let factory = factory()?;
    let dir_str = with_host(|h| h.new_str(dir.to_string_lossy().to_string()));
    host::invoke(&factory, vec![dir_str], None)
}

/// The one-time compiled `require`-closure factory. `require.resolve` /
/// `require.cache` are provided since some packages read them.
fn factory() -> Result<Value, String> {
    if let Some(f) = FACTORY.with(|f| f.borrow().clone()) {
        return Ok(f);
    }
    let src = "var __cjs_factory = (function (__cjs_dir) {\n\
        var req = function (spec) { return __cjs_require(spec, __cjs_dir); };\n\
        req.resolve = function (spec) { return __cjs_resolve(spec, __cjs_dir); };\n\
        req.cache = {};\n\
        req.main = undefined;\n\
        req.extensions = {};\n\
        return req;\n\
    });";
    let f = eval_binding(src, "__cjs_factory")?;
    FACTORY.with(|c| *c.borrow_mut() = Some(f.clone()));
    Ok(f)
}

/// An array of `depth` synthetic V8 CallSite objects for `Error.captureStackTrace`.
/// Stack-introspection packages (e.g. `depd`) set `Error.prepareStackTrace` to a
/// function that receives this array; the getters return neutral placeholders (no
/// real frame info is available), which is enough for those packages to build
/// their deprecation sites without throwing.
pub fn callsite_stack(depth: usize) -> Result<Value, String> {
    let factory = if let Some(f) = CALLSITE_FACTORY.with(|f| f.borrow().clone()) {
        f
    } else {
        let src = "var __cjs_callsites = (function (n) {\n\
            var a = [];\n\
            for (var i = 0; i < n; i++) {\n\
                a.push({\n\
                    getFileName: function () { return null; },\n\
                    getLineNumber: function () { return 0; },\n\
                    getColumnNumber: function () { return 0; },\n\
                    getFunctionName: function () { return null; },\n\
                    getMethodName: function () { return null; },\n\
                    getTypeName: function () { return null; },\n\
                    getThis: function () { return undefined; },\n\
                    isNative: function () { return false; },\n\
                    isEval: function () { return false; },\n\
                    toString: function () { return '<anonymous>'; }\n\
                });\n\
            }\n\
            return a;\n\
        });";
        let f = eval_binding(src, "__cjs_callsites")?;
        CALLSITE_FACTORY.with(|c| *c.borrow_mut() = Some(f.clone()));
        f
    };
    host::invoke(&factory, vec![Value::Float(depth as f64)], None)
}
