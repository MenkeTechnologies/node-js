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
//! `open` returns a `FileHandle`: a port of node's class over the same
//! synchronous descriptor calls (`fs_filehandle.js`), which `readFile`,
//! `writeFile` and `appendFile` also route through when handed one.
//!
//! The work is performed synchronously (node-js has no thread-pooled `fs` at this
//! layer), and the Promise settles in the loop's poll phase, as an I/O
//! completion — ordered with the callback forms' completions, as in node.

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
    "open",
    "opendir",
    "glob",
];

/// The `FileHandle` port (`fs_filehandle.js`), evaluated once: an object with
/// `open`, `FileHandle`, and the handle forms of `readFile` / `writeFile`.
fn file_handle_module() -> Result<Value, String> {
    thread_local! {
        static MODULE: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }
    if let Some(m) = MODULE.with(|m| m.borrow().clone()) {
        return Ok(m);
    }
    let factory = crate::eval_in_global_scope(include_str!("fs_filehandle.js"))?;
    let node_error = with_host(|h| h.alloc(crate::host::JsObj::Builtin("util.@@nodeError".into())));
    let made = crate::host::invoke(&factory, vec![node_error], None)?;
    MODULE.with(|m| *m.borrow_mut() = Some(made.clone()));
    Ok(made)
}

/// Call one of the port's functions by name.
fn call_port(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let f = crate::builtins::get_property(&file_handle_module()?, name)?;
    crate::host::invoke(&f, args, None)
}

/// `path instanceof FileHandle` — the test node's `readFile`/`writeFile` make
/// before treating their first argument as a path. Only an ordinary object can
/// be one; a string never reaches the port.
fn is_file_handle(v: &Value) -> Result<bool, String> {
    let plain = with_host(|h| matches!(h.get(v), Some(crate::host::JsObj::Object(_))));
    if !plain {
        return Ok(false);
    }
    let class = crate::builtins::get_property(&file_handle_module()?, "FileHandle")?;
    let found = crate::host::call_method(&class, "isFileHandle", vec![v.clone()])?;
    Ok(with_host(|h| h.truthy(&found)))
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if method == "open" {
        return Some(call_port("open", args.to_vec()));
    }
    // A `FileHandle` in the path position reads or writes through the handle,
    // from its current position (`readFileHandle` / `writeFileHandle`).
    if matches!(method, "readFile" | "writeFile" | "appendFile") {
        match is_file_handle(args.first().unwrap_or(&Value::Undef)) {
            Ok(true) => {
                let port = if method == "readFile" {
                    "readFile"
                } else {
                    "writeFile"
                };
                return Some(call_port(port, args.to_vec()));
            }
            Ok(false) => {}
            Err(e) => return Some(Err(e)),
        }
    }
    // `options.signal`: validated, and an already-aborted one rejects with
    // node's `AbortError` before any work. It was ignored.
    if matches!(method, "readFile" | "writeFile" | "appendFile") {
        let opts = args.get(if method == "readFile" { 1 } else { 2 });
        if let Some(o) = opts.filter(|o| with_host(|h| h.type_of(o)) == "object") {
            let refusal = match call_port("signalRefusal", vec![o.clone()]) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };
            if !matches!(refusal, Value::Undef) {
                let p = with_host(|h| h.new_promise());
                let id = with_host(|h| h.promise_id(&p).unwrap_or(0));
                crate::host::reject_promise_val(id, refusal);
                return Some(Ok(p));
            }
        }
    }
    // Every other method delegates to the synchronous `fs` sibling of the same
    // name, then wraps the outcome in a settled Promise.
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
    // The error is built now, while the stack it records is the caller's.
    let outcome = match result {
        Ok(v) => Ok(v),
        Err(e) => Err(with_host(|h| crate::builtins::synth_error(h, &e))),
    };
    // node settles the promise when the thread-pool job completes, in the
    // loop's poll phase — the same queue as a callback-form completion. It
    // was settled here and now, so `fs.readFile(p, cb)` started before
    // `fsPromises.readFile(p)` saw its callback run second.
    with_host(|h| {
        h.queue_io_native(Box::new(move || {
            match outcome {
                Ok(v) => crate::host::resolve_promise_val(id, v),
                Err(ev) => crate::host::reject_promise_val(id, ev),
            }
            Ok(())
        }))
    });
    p
}
