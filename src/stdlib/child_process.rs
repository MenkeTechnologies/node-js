//! Node `child_process` module — real subprocess execution via
//! `std::process::Command`.
//!
//! The synchronous entry points (`execSync`, `spawnSync`, `execFileSync`) are
//! fully implemented: they spawn the child with piped stdio, wait for it, and
//! return its captured output. `stdout`/`stderr` are returned as `Buffer`s by
//! default (built through `buffer::from_bytes`, identical to the `fs` module's
//! byte returns) or as strings when an `encoding` other than `"buffer"` is
//! given in the options object.
//!
//! The asynchronous forms are backed synchronously here because node-js has no
//! socket-driven child event loop:
//!   * `exec(cmd, cb)` runs the command to completion, then delivers the result
//!     through its callback `(error, stdout, stderr)` scheduled as a microtask
//!     (`queue_micro`), matching Node's "callback fires after the current tick"
//!     ordering. `stdout`/`stderr` are strings, as Node's `exec` default.
//!   * `spawn(cmd, args)` runs the command to completion up front and returns a
//!     minimal ChildProcess-shaped object carrying the already-collected
//!     `pid`, `exitCode`, `stdout` and `stderr`. LIMITATION: because there is no
//!     event loop at this layer, the returned object is NOT a live EventEmitter
//!     — `.on('close'|'exit'|'data', …)` listeners registered by the caller
//!     after `spawn` returns do not fire (the process has already finished and
//!     its output is exposed as plain properties instead).

use super::arg_str;
use crate::host::with_host;
use fusevm::Value;
use indexmap::IndexMap;
use std::process::{Command, Stdio};

pub const METHODS: &[&str] = &[
    "execSync",
    "spawnSync",
    "execFileSync",
    "exec",
    "spawn",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "execSync" => exec_sync(args),
        "spawnSync" => spawn_sync(args),
        "execFileSync" => exec_file_sync(args),
        "exec" => exec(args),
        "spawn" => spawn(args),
        _ => return None,
    })
}

/// Result of running a child to completion: exit code (`None` if terminated by a
/// signal), captured stdout, captured stderr.
struct Run {
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pid: u32,
}

/// Spawn `program` with `args`, capture both pipes, and wait for exit.
fn run(program: &str, args: &[String]) -> std::io::Result<Run> {
    let child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();
    let out = child.wait_with_output()?;
    Ok(Run {
        status: out.status.code(),
        stdout: out.stdout,
        stderr: out.stderr,
        pid,
    })
}

/// `execSync(command[, options])` — run `sh -c <command>`, return stdout, and
/// throw when the command exits non-zero (matching Node's `execSync`).
fn exec_sync(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let enc = opts_encoding(args, 1);
    let r = run("sh", &["-c".to_string(), cmd.clone()])
        .map_err(|e| format!("Error: {e}"))?;
    if r.status != Some(0) {
        let tail = String::from_utf8_lossy(&r.stderr);
        return Err(format!("Error: Command failed: {cmd}\n{tail}"));
    }
    Ok(output_value(&r.stdout, enc.as_deref()))
}

/// `spawnSync(command, args[, options])` — return
/// `{ status, signal, pid, stdout, stderr }` (never throws on non-zero exit).
fn spawn_sync(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let cmd_args = arg_array(args, 1);
    let enc = opts_encoding(args, 2);
    match run(&cmd, &cmd_args) {
        Ok(r) => {
            // Build the stdout/stderr values FIRST (each allocates via its own
            // `with_host`); inserting them inside the outer `with_host` below would
            // re-enter the host borrow and panic.
            let stdout = output_value(&r.stdout, enc.as_deref());
            let stderr = output_value(&r.stderr, enc.as_deref());
            Ok(with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("pid".into(), Value::Float(r.pid as f64));
                m.insert(
                    "status".into(),
                    r.status.map(|c| Value::Float(c as f64)).unwrap_or_else(|| h.null()),
                );
                // A signal name is not recovered here; report null (as when the
                // child exited normally).
                m.insert("signal".into(), h.null());
                m.insert("stdout".into(), stdout);
                m.insert("stderr".into(), stderr);
                h.new_object(m)
            }))
        }
        // Failure to launch (e.g. ENOENT): Node populates `error` and leaves
        // status/stdout/stderr null.
        Err(e) => Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("pid".into(), Value::Float(0.0));
            m.insert("status".into(), h.null());
            m.insert("signal".into(), h.null());
            m.insert("stdout".into(), h.null());
            m.insert("stderr".into(), h.null());
            m.insert("error".into(), h.new_str(format!("Error: spawn {cmd} {e}")));
            h.new_object(m)
        })),
    }
}

/// `execFileSync(file, args[, options])` — like `spawnSync` but returns stdout
/// and throws on a non-zero exit.
fn exec_file_sync(args: &[Value]) -> Result<Value, String> {
    let file = arg_str(args, 0);
    let cmd_args = arg_array(args, 1);
    let enc = opts_encoding(args, 2);
    let r = run(&file, &cmd_args).map_err(|e| format!("Error: spawn {file} {e}"))?;
    if r.status != Some(0) {
        let tail = String::from_utf8_lossy(&r.stderr);
        return Err(format!("Error: Command failed: {file}\n{tail}"));
    }
    Ok(output_value(&r.stdout, enc.as_deref()))
}

/// `exec(command[, options], callback)` — run `sh -c <command>` synchronously,
/// then fire `callback(error, stdout, stderr)` as a microtask. Node's `exec`
/// defaults to string output, so stdout/stderr are passed as strings.
fn exec(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    // Callback is the last function-shaped argument.
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    let (err, out, errout) = match run("sh", &["-c".to_string(), cmd.clone()]) {
        Ok(r) => {
            let stdout = String::from_utf8_lossy(&r.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&r.stderr).into_owned();
            let err = if r.status == Some(0) {
                with_host(|h| h.null())
            } else {
                let code = r.status.unwrap_or(-1);
                with_host(|h| h.new_str(format!("Error: Command failed: {cmd}\nexit code {code}")))
            };
            (err, stdout, stderr)
        }
        Err(e) => (
            with_host(|h| h.new_str(format!("Error: {e}"))),
            String::new(),
            String::new(),
        ),
    };
    with_host(|h| {
        let so = h.new_str(out);
        let se = h.new_str(errout);
        h.queue_micro(cb, vec![err, so, se]);
    });
    Ok(Value::Undef)
}

/// `spawn(command, args[, options])` — see the module doc comment: runs the
/// child synchronously and returns a minimal, non-live ChildProcess-shaped
/// object exposing the collected result. Event listeners do not fire.
fn spawn(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let cmd_args = arg_array(args, 1);
    match run(&cmd, &cmd_args) {
        Ok(r) => {
            // Allocate the Buffers before the outer `with_host` (from_bytes borrows
            // the host itself — nesting would panic).
            let stdout = super::buffer::from_bytes(&r.stdout);
            let stderr = super::buffer::from_bytes(&r.stderr);
            Ok(with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("pid".into(), Value::Float(r.pid as f64));
                m.insert(
                    "exitCode".into(),
                    r.status.map(|c| Value::Float(c as f64)).unwrap_or_else(|| h.null()),
                );
                m.insert("signalCode".into(), h.null());
                m.insert("killed".into(), Value::Bool(false));
                m.insert("stdout".into(), stdout);
                m.insert("stderr".into(), stderr);
                h.new_object(m)
            }))
        }
        Err(e) => Err(format!("Error: spawn {cmd} {e}")),
    }
}

/// Bytes → a `Buffer` value (default) or a decoded string when `encoding` is set
/// to anything other than `"buffer"`. Buffers are built exactly like `fs`
/// returns them, via `buffer::from_bytes`.
fn output_value(bytes: &[u8], encoding: Option<&str>) -> Value {
    match encoding {
        Some(enc) if !enc.eq_ignore_ascii_case("buffer") => {
            with_host(|h| h.new_str(String::from_utf8_lossy(bytes).into_owned()))
        }
        _ => super::buffer::from_bytes(bytes),
    }
}

/// The array argument at `args[i]` as a list of stringified elements (empty when
/// the argument is absent or not an array).
fn arg_array(args: &[Value], i: usize) -> Vec<String> {
    with_host(|h| match args.get(i).and_then(|v| h.get(v)) {
        Some(crate::host::JsObj::Array(items)) => items.iter().map(|v| h.str_of(v)).collect(),
        _ => Vec::new(),
    })
}

/// Read `.encoding` from the options object at `args[i]`, if present and a
/// non-empty string.
fn opts_encoding(args: &[Value], i: usize) -> Option<String> {
    with_host(|h| match args.get(i).and_then(|v| h.get(v)) {
        Some(crate::host::JsObj::Object(p)) => {
            p.get("encoding").map(|v| h.str_of(v)).filter(|s| !s.is_empty() && s != "undefined" && s != "null")
        }
        _ => None,
    })
}
