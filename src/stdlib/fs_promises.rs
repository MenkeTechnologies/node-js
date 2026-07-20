//! Node `fs/promises` (also `require('fs').promises`) — Promise-returning file
//! operations.
//!
//! Every method delegates to the SAME synchronous I/O the `*Sync` `fs` module
//! performs (`readFile`→`readFileSync`, `chmod`→`chmodSync`, …), then wraps the
//! outcome in a settled Promise: fulfilled with the value on success, rejected
//! with an `Error` object on failure. Delegating to `fs::call` keeps the encoding
//! handling, `Stats`/`Dirent`/`Dir` shapes, and error strings identical to the
//! rest of `fs`.
//!
//! The work is performed synchronously (node-js has no thread-pooled `fs` at this
//! layer); the Promise is already settled when returned, so `await`/`.then`
//! observe the result on the next microtask tick — the observable contract
//! callers rely on.

use crate::host::with_host;
use fusevm::Value;

pub const METHODS: &[&str] = &[
    "readFile",
    "writeFile",
    "appendFile",
    "readdir",
    "mkdir",
    "rmdir",
    "rm",
    "unlink",
    "stat",
    "lstat",
    "statfs",
    "access",
    "rename",
    "copyFile",
    "cp",
    "chmod",
    "chown",
    "lchown",
    "link",
    "symlink",
    "readlink",
    "realpath",
    "truncate",
    "utimes",
    "lutimes",
    "mkdtemp",
    "opendir",
    "glob",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // Every method delegates to the synchronous `fs` sibling of the same name,
    // then wraps the outcome in a settled Promise.
    let sync_name = match method {
        "readFile" => "readFileSync",
        "writeFile" => "writeFileSync",
        "appendFile" => "appendFileSync",
        "readdir" => "readdirSync",
        "mkdir" => "mkdirSync",
        "rmdir" => "rmdirSync",
        "rm" => "rmSync",
        "unlink" => "unlinkSync",
        "stat" => "statSync",
        "lstat" => "lstatSync",
        "statfs" => "statfsSync",
        "access" => "accessSync",
        "rename" => "renameSync",
        "copyFile" => "copyFileSync",
        "cp" => "cpSync",
        "chmod" => "chmodSync",
        "chown" => "chownSync",
        "lchown" => "lchownSync",
        "link" => "linkSync",
        "symlink" => "symlinkSync",
        "readlink" => "readlinkSync",
        "realpath" => "realpathSync",
        "truncate" => "truncateSync",
        "utimes" => "utimesSync",
        "lutimes" => "lutimesSync",
        "mkdtemp" => "mkdtempSync",
        "opendir" => "opendirSync",
        "glob" => "globSync",
        _ => return None,
    };
    // The method itself always succeeds in *returning a Promise*; success/failure
    // of the I/O is encoded in that Promise's settled state.
    Some(Ok(settled(sync(sync_name, args))))
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
