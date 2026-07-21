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
//!   * `execFile(file, args, cb)` is `exec` without a shell — `file` is run
//!     directly with the `args` array — and additionally returns a (non-live)
//!     ChildProcess-shaped object carrying the collected result.
//!   * `spawn(cmd, args)` runs the command to completion up front and returns a
//!     minimal ChildProcess-shaped object carrying the already-collected
//!     `pid`, `exitCode`, `stdout` and `stderr`. LIMITATION: because these run
//!     synchronously, the returned object is not live — `.on('close'|'exit', …)`
//!     listeners registered by the caller after the call do not fire (the
//!     process has already finished and its output is exposed as properties).
//!
//! `fork(modulePath)` is the exception: it spawns THIS `node` executable on
//! `modulePath` as a genuinely live child and returns a live ChildProcess
//! emitter that fires `exit`/`close` when the process terminates and supports
//! `.kill()`. Its IPC channel (`.send()` / `process.on('message')`) is NOT
//! implemented — see the `fork` fn doc for why.

use super::arg_str;
use crate::host::{with_host, IoTask, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const METHODS: &[&str] = &[
    "execSync",
    "spawnSync",
    "execFileSync",
    "exec",
    "execFile",
    "spawn",
    "fork",
];

/// Instance method names for the `ChildProcess` `@@native` tag, exposed to
/// `stdlib::instance_has_method` (property reads that yield a bound method).
pub const CHILD_PROCESS_METHODS: &[&str] = &["kill", "send", "disconnect", "ref", "unref"];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "execSync" => exec_sync(args),
        "spawnSync" => spawn_sync(args),
        "execFileSync" => exec_file_sync(args),
        "exec" => exec(args),
        "execFile" => exec_file(args),
        "spawn" => spawn(args),
        "fork" => fork(args),
        _ => return None,
    })
}

// ── live ChildProcess registry (used by `fork`) ──────────────────────────────

/// Process-global id source for live (`fork`ed) children.
static NEXT_CHILD_ID: AtomicU64 = AtomicU64::new(1);

/// Main-thread record for a live child: its emitter object and a shared handle
/// the waiter thread polls (`try_wait`) and `kill` signals through.
struct ChildRec {
    emitter: Value,
    handle: Arc<Mutex<Option<Child>>>,
}

thread_local! {
    static CHILDREN: std::cell::RefCell<std::collections::HashMap<u64, ChildRec>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Build a `ChildProcess`-shaped emitter object (tagged `@@native = "ChildProcess"`)
/// carrying the given extra properties, sharing the EventEmitter shape.
fn child_object(extra: IndexMap<String, Value>) -> Value {
    super::net::new_emitter_object("ChildProcess", extra)
}

/// Result of running a child to completion: exit code (`None` if terminated by a
/// signal), captured stdout, captured stderr.
struct Run {
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pid: u32,
}

/// Spawn `program` with `args`, capture both pipes, optionally feed `input` to
/// stdin, and wait for exit.
fn run(program: &str, args: &[String], input: Option<&[u8]>) -> std::io::Result<Run> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let mut child = cmd.spawn()?;
    let pid = child.id();
    if let Some(bytes) = input {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write as _;
            let _ = stdin.write_all(bytes);
            // Drop stdin to send EOF so the child (e.g. `cat`/`wc`) can finish.
        }
    }
    let out = child.wait_with_output()?;
    Ok(Run {
        status: out.status.code(),
        stdout: out.stdout,
        stderr: out.stderr,
        pid,
    })
}

/// The `opts.input` (stdin) bytes for a *Sync call, if provided.
fn opts_input(args: &[Value], idx: usize) -> Option<Vec<u8>> {
    let opts = args.get(idx)?;
    match crate::builtins::get_property(opts, "input") {
        Ok(Value::Undef) => None,
        Ok(v) => Some(super::arg_str(&[v], 0).into_bytes()),
        Err(_) => None,
    }
}

/// `execSync(command[, options])` — run `sh -c <command>`, return stdout, and
/// throw when the command exits non-zero (matching Node's `execSync`).
fn exec_sync(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let enc = opts_encoding(args, 1);
    let r = run(
        "sh",
        &["-c".to_string(), cmd.clone()],
        opts_input(args, 1).as_deref(),
    )
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
    match run(&cmd, &cmd_args, opts_input(args, 2).as_deref()) {
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
                    r.status
                        .map(|c| Value::Float(c as f64))
                        .unwrap_or_else(|| h.null()),
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
    let r = run(&file, &cmd_args, opts_input(args, 2).as_deref())
        .map_err(|e| format!("Error: spawn {file} {e}"))?;
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
    let Some(cb) = args.last().cloned() else {
        return Ok(Value::Undef);
    };
    let (err, out, errout) = match run("sh", &["-c".to_string(), cmd.clone()], None) {
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
    match run(&cmd, &cmd_args, None) {
        Ok(r) => {
            // Allocate the Buffers / null before building the map (`from_bytes` and
            // `null` borrow the host — nesting inside another `with_host` panics).
            let stdout = super::buffer::from_bytes(&r.stdout);
            let stderr = super::buffer::from_bytes(&r.stderr);
            let null = with_host(|h| h.null());
            let mut m = IndexMap::new();
            m.insert("pid".into(), Value::Float(r.pid as f64));
            m.insert(
                "exitCode".into(),
                r.status
                    .map(|c| Value::Float(c as f64))
                    .unwrap_or_else(|| null.clone()),
            );
            m.insert("signalCode".into(), null);
            m.insert("killed".into(), Value::Bool(false));
            m.insert("connected".into(), Value::Bool(false));
            m.insert("stdout".into(), stdout);
            m.insert("stderr".into(), stderr);
            Ok(child_object(m))
        }
        Err(e) => Err(format!("Error: spawn {cmd} {e}")),
    }
}

/// `execFile(file[, args][, options][, callback])` — like `exec` but WITHOUT a
/// shell: `file` is run directly with the `args` array. Runs to completion, fires
/// `callback(error, stdout, stderr)` (strings) as a microtask, and returns a
/// (non-live) ChildProcess-shaped object carrying the collected result.
fn exec_file(args: &[Value]) -> Result<Value, String> {
    let file = arg_str(args, 0);
    let cmd_args = arg_array(args, 1);
    // Callback is the last function-shaped argument, if any.
    let cb = args
        .iter()
        .rev()
        .find(|v| with_host(|h| crate::host::is_callable(h, v)))
        .cloned();

    match run(&file, &cmd_args, None) {
        Ok(r) => {
            let stdout_buf = super::buffer::from_bytes(&r.stdout);
            let stderr_buf = super::buffer::from_bytes(&r.stderr);
            let null = with_host(|h| h.null());
            if let Some(cb) = cb {
                let so = String::from_utf8_lossy(&r.stdout).into_owned();
                let se = String::from_utf8_lossy(&r.stderr).into_owned();
                let err = if r.status == Some(0) {
                    null.clone()
                } else {
                    let code = r.status.unwrap_or(-1);
                    with_host(|h| {
                        h.new_str(format!("Error: Command failed: {file}\nexit code {code}"))
                    })
                };
                with_host(|h| {
                    let so = h.new_str(so);
                    let se = h.new_str(se);
                    h.queue_micro(cb, vec![err, so, se]);
                });
            }
            let mut m = IndexMap::new();
            m.insert("pid".into(), Value::Float(r.pid as f64));
            m.insert(
                "exitCode".into(),
                r.status
                    .map(|c| Value::Float(c as f64))
                    .unwrap_or_else(|| null.clone()),
            );
            m.insert("signalCode".into(), null);
            m.insert("killed".into(), Value::Bool(false));
            m.insert("connected".into(), Value::Bool(false));
            m.insert("stdout".into(), stdout_buf);
            m.insert("stderr".into(), stderr_buf);
            Ok(child_object(m))
        }
        Err(e) => {
            if let Some(cb) = cb {
                let msg = with_host(|h| h.new_str(format!("Error: spawn {file} {e}")));
                let empty1 = with_host(|h| h.new_str(""));
                let empty2 = with_host(|h| h.new_str(""));
                with_host(|h| h.queue_micro(cb, vec![msg, empty1, empty2]));
            }
            Err(format!("Error: spawn {file} {e}"))
        }
    }
}

/// `fork(modulePath[, args][, options])` — spawn THIS `node` executable on
/// `modulePath` as a live child (inheriting stdio), returning a live
/// ChildProcess emitter that fires `exit`/`close` when the child terminates.
///
/// LIMITATION: Node's `fork` also opens an IPC channel so parent and child can
/// exchange messages via `child.send()` / `process.on('message')`. That requires
/// the child `node` process to detect and bind an inherited IPC file descriptor,
/// which this runtime does not implement — so `child.send()` is a no-op that
/// returns `false`, `child.connected` is `false`, and no `'message'` event fires.
/// The process itself is real and live (`exit`/`close`/`kill` all work).
fn fork(args: &[Value]) -> Result<Value, String> {
    let module = arg_str(args, 0);
    let extra_args = arg_array(args, 1);
    let exe = std::env::current_exe().map_err(|e| format!("Error: fork: {e}"))?;

    let mut cmd = Command::new(exe);
    cmd.arg(&module).args(&extra_args);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let child = cmd
        .spawn()
        .map_err(|e| format!("Error: fork {module} {e}"))?;
    let pid = child.id();

    let id = NEXT_CHILD_ID.fetch_add(1, Ordering::Relaxed);
    let handle = Arc::new(Mutex::new(Some(child)));

    let mut extra = IndexMap::new();
    extra.insert("@@childid".into(), Value::Float(id as f64));
    extra.insert("pid".into(), Value::Float(pid as f64));
    extra.insert("connected".into(), Value::Bool(false));
    extra.insert("killed".into(), Value::Bool(false));
    extra.insert("exitCode".into(), with_host(|h| h.null()));
    extra.insert("signalCode".into(), with_host(|h| h.null()));
    let emitter = child_object(extra);
    CHILDREN.with(|c| {
        c.borrow_mut().insert(
            id,
            ChildRec {
                emitter: emitter.clone(),
                handle: handle.clone(),
            },
        );
    });
    with_host(|h| h.incr_handle());

    let io_tx = with_host(|h| h.io_sender());
    std::thread::spawn(move || wait_child(id, handle, io_tx));
    Ok(emitter)
}

/// Background waiter for a `fork`ed child: polls `try_wait` (so `kill` can still
/// acquire the shared handle between polls) and posts an `IoTask` emitting
/// `exit`/`close` once the child terminates.
fn wait_child(id: u64, handle: Arc<Mutex<Option<Child>>>, io_tx: Sender<IoTask>) {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let status = {
            let mut g = match handle.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            match g.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        *g = None;
                        Some(status.code())
                    }
                    Ok(None) => None,
                    Err(_) => {
                        *g = None;
                        Some(None)
                    }
                },
                // Handle already taken (killed + reaped elsewhere): stop polling.
                None => return,
            }
        };
        if let Some(code) = status {
            let _ = io_tx.send(Box::new(move || on_child_exit(id, code)));
            return;
        }
    }
}

/// Main-thread handler: emit `exit` then `close` on a terminated child, mark it,
/// release its event-loop handle, and drop its registry record.
fn on_child_exit(id: u64, code: Option<i32>) -> Result<(), String> {
    let emitter = CHILDREN.with(|c| c.borrow().get(&id).map(|r| r.emitter.clone()));
    let Some(emitter) = emitter else {
        return Ok(());
    };
    let (code_val, null1, null2) = with_host(|h| {
        let cv = code
            .map(|c| Value::Float(c as f64))
            .unwrap_or_else(|| h.null());
        (cv, h.null(), h.null())
    });
    set_prop(&emitter, "exitCode", code_val.clone());
    set_prop(&emitter, "killed", Value::Bool(true));
    let ev_exit = with_host(|h| h.new_str("exit"));
    let ev_close = with_host(|h| h.new_str("close"));
    super::events::instance_call(&emitter, "emit", vec![ev_exit, code_val.clone(), null1])?;
    super::events::instance_call(&emitter, "emit", vec![ev_close, code_val, null2])?;
    CHILDREN.with(|c| c.borrow_mut().remove(&id));
    with_host(|h| h.decr_handle());
    let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    Ok(())
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

// ── ChildProcess instance methods (tag `@@native = "ChildProcess"`) ──────────

/// `stdlib::instance_call` entry for a `ChildProcess` receiver. EventEmitter
/// methods delegate to `events`; process-control methods act on the live child
/// (only `fork`ed children are live — a `spawn`/`execFile` result has already
/// exited, so `kill` is a no-op there).
pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        "on"
        | "addListener"
        | "prependListener"
        | "once"
        | "prependOnceListener"
        | "emit"
        | "removeListener"
        | "off"
        | "removeAllListeners"
        | "listenerCount"
        | "eventNames"
        | "setMaxListeners"
        | "getMaxListeners"
        | "listeners" => super::events::instance_call(recv, method, args),
        "kill" => Ok(Value::Bool(kill_child(recv))),
        // IPC is not implemented (see `fork` doc): `send` cannot deliver a message.
        "send" => Ok(Value::Bool(false)),
        "disconnect" => {
            set_prop(recv, "connected", Value::Bool(false));
            Ok(Value::Undef)
        }
        "ref" | "unref" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "child.{method} is not a function"
        ))),
    }
}

/// Terminate a live (`fork`ed) child. The signal argument is accepted for API
/// compatibility but ignored — `std::process::Child::kill` always sends `SIGKILL`.
/// Returns `true` if a live child was signalled.
fn kill_child(recv: &Value) -> bool {
    let id = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@childid").map(|v| h.to_number(v) as u64),
        _ => None,
    });
    let Some(id) = id else { return false };
    let handle = CHILDREN.with(|c| c.borrow().get(&id).map(|r| r.handle.clone()));
    let Some(handle) = handle else { return false };
    if let Ok(mut g) = handle.lock() {
        if let Some(child) = g.as_mut() {
            let _ = child.kill();
            return true;
        }
    }
    false
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
        Some(crate::host::JsObj::Object(p)) => p
            .get("encoding")
            .map(|v| h.str_of(v))
            .filter(|s| !s.is_empty() && s != "undefined" && s != "null"),
        _ => None,
    })
}
