//! Node `process` global — the subset packages read at load time.
//!
//! Data properties (`process.env`, `process.argv`, `process.platform`, the
//! `stdout`/`stderr` stream stand-ins, …) are served through `constant`;
//! callable members (`process.cwd()`, `process.hrtime()`, the EventEmitter-style
//! `on`/`emit` no-ops, …) through `call`. `process.nextTick` is intentionally NOT
//! handled here — it stays on the core microtask path in `builtins.rs`.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Callable members. `nextTick` is deliberately absent (handled in `builtins`).
pub const METHODS: &[&str] = &[
    "cwd",
    "chdir",
    "exit",
    "hrtime",
    "uptime",
    "memoryUsage",
    "cpuUsage",
    "umask",
    "binding",
    "emit",
    "on",
    "once",
    "off",
    "addListener",
    "removeListener",
    "removeAllListeners",
    "listeners",
    "emitWarning",
    "kill",
];

/// Data properties, served through `namespace_property` → `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "env" => env_object(),
        "argv" => argv(),
        "argv0" => with_host(|h| h.new_str(exec_path())),
        "execPath" => with_host(|h| h.new_str(exec_path())),
        "execArgv" => with_host(|h| h.new_array(Vec::new())),
        "platform" => with_host(|h| h.new_str(super::os::platform())),
        "arch" => with_host(|h| h.new_str(super::os::arch())),
        "pid" => Value::Float(std::process::id() as f64),
        "ppid" => Value::Float(0.0),
        "title" => with_host(|h| h.new_str("node")),
        // A best-effort Node-compatible version string. Kept low so a dep's
        // `if (semver.lt(process.version, ...))` gate takes the conservative path.
        "version" => with_host(|h| h.new_str("v26.5.0")),
        "versions" => versions(),
        "stdout" => std_stream(1),
        "stderr" => std_stream(2),
        "stdin" => std_stream(0),
        _ => return None,
    })
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "cwd" => {
            let d = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(with_host(|h| h.new_str(d)))
        }
        // `hrtime()` → `[seconds, nanoseconds]` since an arbitrary epoch (here the
        // monotonic clock via `Instant` is unavailable statically, so use the
        // system clock — sufficient for the timing scaffolding deps set up).
        "hrtime" => Ok(hrtime(args)),
        "uptime" => Ok(Value::Float(0.0)),
        "memoryUsage" => Ok(memory_usage()),
        "cpuUsage" => Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("user".into(), Value::Float(0.0));
            m.insert("system".into(), Value::Float(0.0));
            h.new_object(m)
        })),
        "umask" => Ok(Value::Float(0.0)),
        "binding" => Err(crate::host::type_error("process.binding is not supported")),
        // EventEmitter-style registration is accepted and ignored (returns the
        // process namespace so `.on(...).on(...)` chains work); no signals fire.
        "on" | "once" | "off" | "addListener" | "removeListener" | "removeAllListeners" => {
            Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into()))))
        }
        "listeners" => Ok(with_host(|h| h.new_array(Vec::new()))),
        "emit" => Ok(Value::Bool(false)),
        "emitWarning" => Ok(Value::Undef),
        "chdir" | "exit" | "kill" => Ok(Value::Undef),
        _ => return None,
    })
}

/// `process.env` as a plain object built from the real environment.
fn env_object() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in std::env::vars() {
            m.insert(k, h.new_str(v));
        }
        h.new_object(m)
    })
}

/// `process.argv`: `[execPath, entryScript, ...userArgs]` (best effort).
fn argv() -> Value {
    with_host(|h| {
        let items: Vec<Value> = std::env::args().map(|a| h.new_str(a)).collect();
        h.new_array(items)
    })
}

fn exec_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "node".into())
}

/// `process.versions` — a small map; only `node` is commonly gated on.
fn versions() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("node".into(), h.new_str("26.5.0"));
        m.insert("v8".into(), h.new_str("0.0.0"));
        h.new_object(m)
    })
}

/// A minimal `process.stdout`/`stderr`/`stdin` stand-in: enough surface
/// (`fd`, `isTTY`, `writable`, a `write`) for load-time probes like
/// `tty.isatty(process.stderr.fd)`.
fn std_stream(fd: i32) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("fd".into(), Value::Float(fd as f64));
        // SAFETY: isatty is a pure query on the fd number.
        let is_tty = unsafe { libc::isatty(fd) == 1 };
        m.insert("isTTY".into(), Value::Bool(is_tty));
        m.insert("writable".into(), Value::Bool(fd != 0));
        m.insert("readable".into(), Value::Bool(fd == 0));
        h.new_object(m)
    })
}

fn hrtime(args: &[Value]) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (mut secs, mut nanos) = (now.as_secs() as f64, now.subsec_nanos() as f64);
    // `hrtime(prev)` returns the diff from a prior reading.
    if let Some(Value::Obj(_)) = args.first() {
        if let Some(prev) = with_host(|h| match h.get(&args[0]) {
            Some(JsObj::Array(a)) if a.len() == 2 => Some((h.to_number(&a[0]), h.to_number(&a[1]))),
            _ => None,
        }) {
            secs -= prev.0;
            nanos -= prev.1;
        }
    }
    with_host(|h| h.new_array(vec![Value::Float(secs), Value::Float(nanos)]))
}

fn memory_usage() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for k in ["rss", "heapTotal", "heapUsed", "external", "arrayBuffers"] {
            m.insert(k.into(), Value::Float(0.0));
        }
        h.new_object(m)
    })
}
