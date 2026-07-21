//! Node `module` core module — `require('module')` (a.k.a. `require('node:module')`).
//!
//! This is DISTINCT from `src/module.rs` (the CommonJS loader that actually reads
//! and runs files). This file is the user-facing `module` namespace: `isBuiltin`,
//! `builtinModules`, `createRequire`, `Module.wrap`, etc. The heavy lifting reuses
//! the loader's own machinery — `createRequire` mints a real dir-bound `require`
//! closure through the same `__cjs_require`/`__cjs_resolve` global dispatch the
//! loader's per-module `require` uses (see `src/module.rs`).
//!
//! `require('module').Module` is the `Module` class namespace; its statics
//! (`Module.isBuiltin`, `Module.wrap`, `Module.createRequire`, `Module.builtinModules`)
//! delegate to the same implementations as the module-level exports (Node's
//! `Module.<x> === module.<x>` for these).

use crate::host::{with_host, JsObj};
use fusevm::Value;

/// Free functions exported by `require('module')`.
pub const METHODS: &[&str] = &[
    "isBuiltin",
    "createRequire",
    "wrap",
    "syncBuiltinESMExports",
    "runMain",
    "findPackageJSON",
];

/// Static method names on the `Module` class (same surface as the module-level
/// exports — Node aliases them).
pub const MODULE_STATIC_METHODS: &[&str] = METHODS;

/// The canonical `module.builtinModules` list: the specifiers `stdlib::resolve`
/// accepts (the modules node-js actually provides), plus the two modules added in
/// this batch (`module`, `stream/consumers`). Sorted, no `node:` duplicates and
/// no hidden aliases (`sys`), matching how Node presents `builtinModules`.
const BUILTIN_MODULES: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "stream/consumers",
    "string_decoder",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// The `createRequire` factory: builds a real dir-bound `require` closure (the
/// same shape as the loader's per-module `require`, with `.resolve`/`.cache`/…),
/// resolving against `path.dirname(referencingPath)`. A `file:` URL argument is
/// converted with `url.fileURLToPath` first.
const CREATE_REQUIRE_SRC: &str = "(function (p) {\n\
  if (typeof p !== 'string') { p = String(p); }\n\
  if (p.indexOf('file://') === 0) { p = require('url').fileURLToPath(p); }\n\
  var dir = require('path').dirname(p);\n\
  var req = function (spec) { return __cjs_require(spec, dir); };\n\
  req.resolve = function (spec) { return __cjs_resolve(spec, dir); };\n\
  req.cache = {};\n\
  req.main = undefined;\n\
  req.extensions = {};\n\
  return req;\n\
})";

/// Module free-function dispatch (`module.isBuiltin`, `module.createRequire`, …).
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "isBuiltin" => Ok(is_builtin(args)),
        "createRequire" => create_require(args),
        "wrap" => Ok(wrap(args)),
        // No ESM/CJS live-binding sync in this runtime — accept and no-op.
        "syncBuiltinESMExports" => Ok(Value::Undef),
        // The entry script is already run by the host; a programmatic runMain is a
        // best-effort no-op.
        "runMain" => Ok(Value::Undef),
        "findPackageJSON" => Ok(find_package_json(args)),
        _ => return None,
    })
}

/// `Module.<method>` static dispatch — same implementations as the module-level
/// exports.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    call(method, args)
}

/// Non-function exports of `require('module')`: `builtinModules` (array) and the
/// `Module` class namespace.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "builtinModules" => Some(builtin_modules_array()),
        "Module" => Some(with_host(|h| h.alloc(JsObj::Builtin("Module".into())))),
        _ => None,
    }
}

/// Non-function statics on the `Module` class: `Module.builtinModules` and the
/// self-referential `Module.Module` (Node's `Module.Module === Module`).
pub fn static_constant(name: &str) -> Option<Value> {
    match name {
        "builtinModules" => Some(builtin_modules_array()),
        "Module" => Some(with_host(|h| h.alloc(JsObj::Builtin("Module".into())))),
        _ => None,
    }
}

// ── implementations ─────────────────────────────────────────────────────────

/// `module.isBuiltin(name)` — true if `name` (with an optional `node:` prefix)
/// names a core module. Any `node:`-prefixed specifier is treated as builtin
/// (matches Node, which reserves the whole `node:` scheme).
fn is_builtin(args: &[Value]) -> Value {
    let name = super::arg_str(args, 0);
    let base = name.strip_prefix("node:").unwrap_or(&name);
    Value::Bool(crate::stdlib::resolve(base).is_some() || name.starts_with("node:"))
}

/// `module.createRequire(filename)` — a `require` bound to `filename`'s directory.
fn create_require(args: &[Value]) -> Result<Value, String> {
    let factory = run_completion(CREATE_REQUIRE_SRC)?;
    let p = args.first().cloned().unwrap_or(Value::Undef);
    crate::host::invoke(&factory, vec![p], None)
}

/// `module.wrap(source)` / `Module.wrap(source)` — the canonical CommonJS module
/// wrapper string Node returns.
fn wrap(args: &[Value]) -> Value {
    let src = super::arg_str(args, 0);
    with_host(|h| {
        h.new_str(format!(
            "(function (exports, require, module, __filename, __dirname) {{ {src}\n}});"
        ))
    })
}

/// `module.findPackageJSON(specifier[, base])` — best-effort: resolve `specifier`
/// (relative to `base`'s directory when given, else cwd), then walk parent
/// directories for the nearest `package.json`. Returns its absolute path, or
/// `undefined` if none is found. `file:` URLs are accepted.
fn find_package_json(args: &[Value]) -> Value {
    use std::path::{Path, PathBuf};
    let strip = |s: String| {
        s.strip_prefix("file://")
            .map(|x| x.to_string())
            .unwrap_or(s)
    };
    let spec = strip(super::arg_str(args, 0));
    let base = if args.len() > 1 {
        Some(strip(super::arg_str(args, 1)))
    } else {
        None
    };
    let start: PathBuf = {
        let p = Path::new(&spec);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(b) = base.as_deref() {
            let bp = Path::new(b);
            let bdir = if bp.is_dir() { bp } else { bp.parent().unwrap_or(bp) };
            bdir.join(&spec)
        } else {
            std::env::current_dir().unwrap_or_default().join(&spec)
        }
    };
    let mut dir = if start.is_dir() {
        Some(start.as_path())
    } else {
        start.parent()
    };
    while let Some(d) = dir {
        let cand = d.join("package.json");
        if cand.is_file() {
            return with_host(|h| h.new_str(cand.to_string_lossy().to_string()));
        }
        dir = d.parent();
    }
    Value::Undef
}

/// Build the `builtinModules` array value from `BUILTIN_MODULES`.
fn builtin_modules_array() -> Value {
    with_host(|h| {
        let items: Vec<Value> = BUILTIN_MODULES.iter().map(|s| h.new_str(*s)).collect();
        h.new_array(items)
    })
}

/// Compile a single JS expression and run it on the LIVE host, returning its
/// completion value (mirrors `util`'s promisify factory path).
fn run_completion(src: &str) -> Result<Value, String> {
    let prog = crate::compile_completion(src)?;
    let chunk = crate::load_merged(prog);
    crate::host::run_chunk_on(chunk)
}
