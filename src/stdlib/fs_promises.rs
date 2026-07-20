//! Node `fs/promises` (also `require('fs').promises`) — Promise-returning file
//! operations.
//!
//! Every method delegates to the SAME synchronous I/O the callback/`*Sync` `fs`
//! module performs, then wraps the outcome in a settled Promise: a fulfilled
//! Promise carrying the value on success, a rejected Promise carrying an `Error`
//! object on failure. Where a synchronous sibling already exists in `fs`
//! (`readFile`→`readFileSync`, etc.) this calls straight into `fs::call`, so the
//! encoding handling, `Stats` shape, and error strings match the rest of `fs`
//! exactly. The three methods `fs` has no sync form for (`access`, `rename`,
//! `copyFile`) do their `std::fs` I/O here directly, formatting errors in the
//! same `Error: <CODE>: <reason>, <op> '<path>'` style.
//!
//! The work is performed synchronously (node-js has no thread-pooled `fs` at this
//! layer); the Promise is already settled when returned, so `await`/`.then`
//! observe the result on the next microtask tick — the observable contract
//! callers rely on.

use super::arg_str;
use crate::host::with_host;
use fusevm::Value;

pub const METHODS: &[&str] = &[
    "readFile",
    "writeFile",
    "appendFile",
    "readdir",
    "mkdir",
    "rmdir",
    "unlink",
    "stat",
    "access",
    "rename",
    "copyFile",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let promise = match method {
        // Delegate to the existing synchronous `fs` implementation.
        "readFile" => settled(sync("readFileSync", args)),
        "writeFile" => settled(sync("writeFileSync", args)),
        "appendFile" => settled(sync("appendFileSync", args)),
        "readdir" => settled(sync("readdirSync", args)),
        "mkdir" => settled(sync("mkdirSync", args)),
        "rmdir" => settled(sync("rmdirSync", args)),
        "unlink" => settled(sync("unlinkSync", args)),
        "stat" => settled(sync("statSync", args)),
        // No synchronous sibling in `fs`; perform the I/O directly.
        "access" => settled(access(args)),
        "rename" => settled(rename(args)),
        "copyFile" => settled(copy_file(args)),
        _ => return None,
    };
    // The method itself always succeeds in *returning a Promise*; success/failure
    // of the I/O is encoded in that Promise's settled state.
    Some(Ok(promise))
}

/// Run a synchronous `fs` method by name, returning its `Result` (the `Option`
/// is always `Some` for the known sync names above).
fn sync(sync_method: &str, args: &[Value]) -> Result<Value, String> {
    super::fs::call(sync_method, args).unwrap_or_else(|| Err("Error: EIO: internal error".into()))
}

/// Wrap a synchronous outcome in an already-settled Promise: fulfilled with the
/// value, or rejected with an `Error` synthesized from the message. The host
/// borrow is released before `resolve`/`reject` (which re-enter the host).
fn settled(result: Result<Value, String>) -> Value {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap_or(0));
    match result {
        Ok(v) => crate::host::resolve_promise_val(id, v),
        Err(e) => {
            let ev = with_host(|h| crate::builtins::synth_error(h, &e));
            crate::host::reject_promise_val(id, ev);
        }
    }
    p
}

/// `fs.promises.access(path[, mode])` — resolves if the path is accessible,
/// rejects otherwise. The `mode` bitmask is not checked (node-js does not model
/// per-bit permission probing); existence is verified via `metadata`.
fn access(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::metadata(&path) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("access", &path, &e)),
    }
}

/// `fs.promises.rename(oldPath, newPath)`.
fn rename(args: &[Value]) -> Result<Value, String> {
    let from = arg_str(args, 0);
    let to = arg_str(args, 1);
    match std::fs::rename(&from, &to) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("rename", &from, &e)),
    }
}

/// `fs.promises.copyFile(src, dest[, mode])` — the `mode` flags are ignored.
fn copy_file(args: &[Value]) -> Result<Value, String> {
    let src = arg_str(args, 0);
    let dest = arg_str(args, 1);
    match std::fs::copy(&src, &dest) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("copyFile", &src, &e)),
    }
}

/// Format an I/O error like `fs`'s own `err_str` (`Error: <CODE>: <reason>, <op>
/// '<path>'`), so `fs/promises` rejections match `fs` sync throws.
fn err_str(op: &str, path: &str, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    let code = match e.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        _ => "EIO",
    };
    format!(
        "Error: {code}: {}, {op} '{path}'",
        e.to_string().split(" (os error").next().unwrap_or("error")
    )
}
