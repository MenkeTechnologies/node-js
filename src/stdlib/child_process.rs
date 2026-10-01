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
//!   * `spawn(cmd, args)` runs the command to completion up front, then
//!     delivers what happened on a later turn of the loop, as events on the
//!     ChildProcess it returned: `'spawn'`, `'data'`/`'end'` on the
//!     `child.stdout`/`child.stderr` streams (or the output written straight
//!     through under `stdio: 'inherit'`), `'exit'`, `'close'`; a binary that
//!     cannot start is an `'error'` event. LIMITATION: the child has finished
//!     before `spawn` returns, so a long-running one blocks the caller and its
//!     output arrives as one chunk per stream.
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
        "@@childEvents" => {
            let child = args.first().cloned().unwrap_or(Value::Undef);
            child_events(&child).map(|_| Value::Undef)
        }
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
    /// The signal that terminated the child, when one did.
    signal: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pid: u32,
}

/// The options a *Sync call passes through to the child.
#[derive(Default)]
struct SpawnOpts {
    input: Option<Vec<u8>>,
    /// `env` REPLACES the environment rather than extending it, as in node.
    env: Option<Vec<(String, String)>>,
    cwd: Option<String>,
}

/// Read `input`, `env` and `cwd` out of the options argument.
///
/// `env` and `cwd` were being ignored entirely: the child inherited this
/// process's environment and working directory, so `spawnSync(cmd, args,
/// { cwd })` silently ran somewhere else and `{ env }` silently saw the wrong
/// variables.
fn spawn_opts(args: &[Value], idx: usize) -> SpawnOpts {
    let Some(opts) = args.get(idx) else {
        return SpawnOpts::default();
    };
    let read = |k: &str| crate::builtins::get_property(opts, k).ok();
    let input = match read("input") {
        Some(Value::Undef) | None => None,
        Some(v) => Some(super::arg_str(&[v], 0).into_bytes()),
    };
    let cwd = match read("cwd") {
        Some(Value::Undef) | None => None,
        Some(v) => Some(with_host(|h| h.str_of(&v))),
    };
    let env = match read("env") {
        Some(v) if with_host(|h| matches!(h.get(&v), Some(JsObj::Object(_)))) => {
            let keys = with_host(|h| match h.get(&v) {
                Some(JsObj::Object(m)) => m
                    .keys()
                    .filter(|k| !k.starts_with("@@"))
                    .cloned()
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            });
            Some(
                keys.into_iter()
                    .filter_map(|k| {
                        let val = crate::builtins::get_property(&v, &k).ok()?;
                        Some((k, with_host(|h| h.str_of(&val))))
                    })
                    .collect(),
            )
        }
        _ => None,
    };
    SpawnOpts { input, env, cwd }
}

/// Spawn `program` with `args`, capture both pipes, optionally feed `input` to
/// stdin, and wait for exit.
fn run(program: &str, args: &[String], opts: &SpawnOpts) -> std::io::Result<Run> {
    let input = opts.input.as_deref();
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = &opts.cwd {
        cmd.current_dir(dir);
    }
    if let Some(vars) = &opts.env {
        cmd.env_clear();
        for (k, v) in vars {
            cmd.env(k, v);
        }
    }
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
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&out.status);
    #[cfg(not(unix))]
    let signal = None;
    Ok(Run {
        status: out.status.code(),
        signal,
        stdout: out.stdout,
        stderr: out.stderr,
        pid,
    })
}

/// `execSync`/`execFileSync` send the child's stderr on to the PARENT's stderr
/// as well as capturing it — that is their documented DEFAULT stdio, and it is
/// how a build script's diagnostics reach the terminal. `spawnSync` does not,
/// and must not.
///
/// "Default" is the operative word: node echoes only when the caller left
/// `stdio` unspecified. Echoing regardless meant a caller that had asked for
/// the pipes explicitly still saw the child's stderr on its own.
fn echo_stderr(args: &[Value], opts_idx: usize, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let explicit_stdio = args
        .get(opts_idx)
        .and_then(|o| crate::builtins::get_property(o, "stdio").ok())
        .is_some_and(|v| !matches!(v, Value::Undef));
    if explicit_stdio {
        return;
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    with_host(|h| h.write_out(&text, true));
}

/// The error a failing `execSync` throws.
///
/// Node throws a real Error carrying `status`, `signal`, `pid`, `stdout` and
/// `stderr`, and the standard shape of a caller is to read `e.status` or
/// `e.stderr`. This used to throw a bare message string, so every one of those
/// read back as undefined and the exit code was unrecoverable.
fn command_failed(cmd: &str, r: &Run, enc: Option<&str>) -> String {
    let tail = String::from_utf8_lossy(&r.stderr).into_owned();
    let msg = format!("Command failed: {cmd}\n{tail}");
    // Build the pipe values before the allocating `with_host` below; each takes
    // its own borrow.
    let stdout = output_value(&r.stdout, enc);
    let stderr = output_value(&r.stderr, enc);
    // Each of these takes its own host borrow, so none may be built inside
    // another's `with_host` closure.
    let e = crate::builtins::make_error_pub("Error", &msg);
    let null = with_host(|h| h.null());
    let status = r
        .status
        .map(|c| Value::Float(c as f64))
        .unwrap_or_else(|| null.clone());
    for (k, v) in [
        ("status", status),
        ("signal", null),
        ("pid", Value::Float(r.pid as f64)),
        ("stdout", stdout),
        ("stderr", stderr),
    ] {
        let _ = crate::builtins::set_property_pub(&e, k, v);
    }
    with_host(|h| h.exc = Some(e));
    format!("Error: {msg}")
}

/// The Error `exec`'s callback is handed for a non-zero exit: node's message is
/// `Command failed: <cmd>\n<stderr>`, and the fields are `cmd`, `code` (the
/// exit status), `killed` and `signal` — nothing else.
fn exec_error(cmd: &str, r: &Run) -> Value {
    let tail = String::from_utf8_lossy(&r.stderr).into_owned();
    let e = crate::builtins::make_error_pub("Error", &format!("Command failed: {cmd}\n{tail}"));
    let null = with_host(|h| h.null());
    let cmd_v = with_host(|h| h.new_str(cmd.to_string()));
    for (k, v) in [
        ("killed", Value::Bool(false)),
        ("code", Value::Float(r.status.unwrap_or(-1) as f64)),
        ("signal", null),
        ("cmd", cmd_v),
    ] {
        let _ = crate::builtins::set_property_pub(&e, k, v);
    }
    e
}

/// The Error a failed SPAWN is reported with — node never throws here, it hands
/// the error-first callback an `ENOENT` carrying `errno`, `syscall`, `path` and
/// `spawnargs`, so `err.code === 'ENOENT'` distinguishes "no such binary" from
/// "the binary ran and failed".
fn spawn_error(file: &str, argv: &[String], e: &std::io::Error) -> Value {
    let code = super::fs::libuv_code(e);
    let err = crate::builtins::make_error_pub("Error", &format!("spawn {file} {code}"));
    let errno = -f64::from(e.raw_os_error().unwrap_or(5));
    let (code_v, syscall, path, spawnargs) = with_host(|h| {
        let items = argv.iter().map(|a| h.new_str(a.clone())).collect();
        (
            h.new_str(code.to_string()),
            h.new_str(format!("spawn {file}")),
            h.new_str(file.to_string()),
            h.new_array(items),
        )
    });
    for (k, v) in [
        ("errno", Value::Float(errno)),
        ("code", code_v),
        ("syscall", syscall),
        ("path", path),
        ("spawnargs", spawnargs),
    ] {
        let _ = crate::builtins::set_property_pub(&err, k, v);
    }
    err
}

/// `execSync(command[, options])` — run `sh -c <command>`, return stdout, and
/// throw when the command exits non-zero (matching Node's `execSync`).
fn exec_sync(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let enc = opts_encoding(args, 1);
    let r = run("sh", &["-c".to_string(), cmd.clone()], &spawn_opts(args, 1))
        .map_err(|e| format!("Error: {e}"))?;
    echo_stderr(args, 1, &r.stderr);
    if r.status != Some(0) {
        return Err(command_failed(&cmd, &r, enc.as_deref()));
    }
    Ok(output_value(&r.stdout, enc.as_deref()))
}

/// `spawnSync(command, args[, options])` — return
/// `{ status, signal, pid, stdout, stderr }` (never throws on non-zero exit).
fn spawn_sync(args: &[Value]) -> Result<Value, String> {
    let cmd = arg_str(args, 0);
    let cmd_args = arg_array(args, 1);
    let enc = opts_encoding(args, 2);
    match run(&cmd, &cmd_args, &spawn_opts(args, 2)) {
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
    let r = run(&file, &cmd_args, &spawn_opts(args, 2))
        .map_err(|e| format!("Error: spawn {file} {e}"))?;
    echo_stderr(args, 2, &r.stderr);
    if r.status != Some(0) {
        // Same rich error `execSync` throws: a caller reads `e.status` and
        // `e.stderr` here exactly as it does there, and this path was still
        // handing back a bare message string.
        return Err(command_failed(&file, &r, enc.as_deref()));
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
    let (err, out, errout) = match run("sh", &["-c".to_string(), cmd.clone()], &spawn_opts(args, 1))
    {
        Ok(r) => {
            let stdout = String::from_utf8_lossy(&r.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&r.stderr).into_owned();
            // An error-first callback receives an ERROR OBJECT carrying node's
            // fields — `err.code` is the exit STATUS, and `cmd`, `killed` and
            // `signal` ride along. This handed over a message string, so
            // `err.code` was `undefined` and the exit status was unrecoverable;
            // the message was this module's own wording too, where node appends
            // the command's STDERR.
            let err = if r.status == Some(0) {
                with_host(|h| h.null())
            } else {
                exec_error(&cmd, &r)
            };
            (err, stdout, stderr)
        }
        Err(e) => (
            with_host(|h| crate::builtins::synth_error(h, &format!("Error: {e}"))),
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
    let disp = stdio_dispositions(args.get(2));
    let null = with_host(|h| h.null());
    let pipe = |d: Disposition| match d {
        Disposition::Pipe => child_pipe(),
        _ => null.clone(),
    };
    let (stdout, stderr) = (pipe(disp[1]), pipe(disp[2]));
    let mut m = IndexMap::new();
    let (status, signal, out, err, spawn_err) = match run(&cmd, &cmd_args, &spawn_opts(args, 2)) {
        Ok(r) => {
            m.insert("pid".into(), Value::Float(r.pid as f64));
            (r.status, r.signal, r.stdout, r.stderr, None)
        }
        // A binary that cannot be started is an `'error'` EVENT, followed by
        // `'close'` with code -2, not a throw: `spawn` is asynchronous in node.
        Err(e) => {
            m.insert("pid".into(), Value::Undef);
            (
                None,
                None,
                Vec::new(),
                Vec::new(),
                Some(spawn_error(&cmd, &cmd_args, &e)),
            )
        }
    };
    // Not exited yet, as far as the caller can tell: `exitCode` is `null` until
    // `'exit'` fires.
    m.insert("exitCode".into(), null.clone());
    m.insert("signalCode".into(), null.clone());
    m.insert("killed".into(), Value::Bool(false));
    m.insert("connected".into(), Value::Bool(false));
    m.insert("stdout".into(), stdout);
    m.insert("stderr".into(), stderr);
    let child = child_object(m);
    PENDING.with(|p| {
        p.borrow_mut().push(Finished {
            child: child.clone(),
            status,
            signal,
            out,
            err,
            disp,
            spawn_err,
        })
    });
    with_host(|h| {
        let cb = h.alloc(JsObj::Builtin("child_process.@@childEvents".into()));
        h.add_timer(-1.0, cb, vec![child.clone()], None);
    });
    Ok(child)
}

/// Where a spawned child's stdout or stderr goes (`options.stdio`).
#[derive(Clone, Copy, PartialEq)]
enum Disposition {
    /// A `child.stdout`/`child.stderr` stream carries it.
    Pipe,
    /// Straight to this process's own stream; the property is `null`.
    Inherit,
    Ignore,
}

/// Read `options.stdio`: one string for all three streams, or an array of
/// per-stream entries, where a descriptor number or a stream object
/// (`process.stdout`) means the parent's own.
fn stdio_dispositions(opts: Option<&Value>) -> [Disposition; 3] {
    let Some(stdio) = opts.and_then(|o| crate::builtins::get_property(o, "stdio").ok()) else {
        return [Disposition::Pipe; 3];
    };
    let one = |v: &Value| -> Disposition {
        match v {
            Value::Float(_) | Value::Int(_) => Disposition::Inherit,
            Value::Undef => Disposition::Pipe,
            _ => with_host(|h| {
                if h.is_null(v) {
                    return Disposition::Pipe;
                }
                if matches!(h.get(v), Some(JsObj::Object(_))) {
                    return Disposition::Inherit;
                }
                match h.str_of(v).as_str() {
                    "inherit" => Disposition::Inherit,
                    "ignore" => Disposition::Ignore,
                    _ => Disposition::Pipe,
                }
            }),
        }
    };
    let items = with_host(|h| match h.get(&stdio) {
        Some(JsObj::Array(items)) => Some(items.clone()),
        _ => None,
    });
    match items {
        Some(items) => {
            let at = |i: usize| one(items.get(i).unwrap_or(&Value::Undef));
            [at(0), at(1), at(2)]
        }
        None => [one(&stdio); 3],
    }
}

/// A finished child whose events have not been delivered yet.
struct Finished {
    child: Value,
    status: Option<i32>,
    signal: Option<i32>,
    out: Vec<u8>,
    err: Vec<u8>,
    disp: [Disposition; 3],
    spawn_err: Option<Value>,
}

thread_local! {
    static PENDING: std::cell::RefCell<Vec<Finished>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A `child.stdout`/`child.stderr` stream: an emitter with the readable
/// surface programs use on it.
fn child_pipe() -> Value {
    super::net::new_emitter_object("ChildPipe", IndexMap::new())
}

/// The methods of a `ChildPipe`, besides the EventEmitter ones.
pub const CHILD_PIPE_METHODS: &[&str] = &["setEncoding", "pipe", "resume", "pause", "destroy"];

pub fn pipe_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if !CHILD_PIPE_METHODS.contains(&method) && super::events::METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        "setEncoding" => {
            let enc = match args.first() {
                Some(v) if !matches!(v, Value::Undef) => arg_str(&args, 0),
                _ => "utf8".to_string(),
            };
            set_prop(recv, "@@encoding", with_host(|h| h.new_str(enc)));
            Ok(recv.clone())
        }
        "pipe" => {
            let dest = args.first().cloned().unwrap_or(Value::Undef);
            set_prop(recv, "@@pipeDest", dest.clone());
            Ok(dest)
        }
        "resume" | "pause" | "destroy" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

fn prop(v: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn emit(target: &Value, event: &str, mut args: Vec<Value>) -> Result<(), String> {
    args.insert(0, with_host(|h| h.new_str(event)));
    super::events::instance_call(target, "emit", args).map(|_| ())
}

/// Deliver a pipe's bytes: one `'data'` chunk (a Buffer, or a string after
/// `setEncoding`), written on to a `pipe` destination too, then `'end'`.
fn pipe_output(pipe: &Value, bytes: &[u8]) -> Result<(), String> {
    let dest = prop(pipe, "@@pipeDest");
    if !bytes.is_empty() {
        let enc = prop(pipe, "@@encoding").map(|v| with_host(|h| h.str_of(&v)));
        let chunk = output_value(bytes, enc.as_deref());
        if let Some(dest) = &dest {
            crate::host::call_method(dest, "write", vec![chunk.clone()])?;
        }
        emit(pipe, "data", vec![chunk])?;
    }
    if let Some(dest) = &dest {
        let is_std = super::native_tag(dest).as_deref() == Some("WriteStream");
        if !is_std {
            crate::host::call_method(dest, "end", Vec::new())?;
        }
    }
    emit(pipe, "end", Vec::new())
}

/// The events of a spawned child, delivered on a later turn of the loop as
/// node delivers them: `'spawn'`, each pipe's output, `'exit'` (with
/// `exitCode`/`signalCode` set), `'close'`, then the pipes' own `'close'`. A
/// child that could not start emits `'error'` and `'close'` with code -2.
fn child_events(child: &Value) -> Result<(), String> {
    let found = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let i = p
            .iter()
            .position(|f| with_host(|h| h.strict_eq(&f.child, child)))?;
        Some(p.remove(i))
    });
    let Some(f) = found else { return Ok(()) };
    let null = with_host(|h| h.null());
    if let Some(err) = f.spawn_err {
        emit(child, "error", vec![err])?;
        return emit(child, "close", vec![Value::Float(-2.0), null]);
    }
    emit(child, "spawn", Vec::new())?;
    let pipes = [prop(child, "stdout"), prop(child, "stderr")];
    for (i, bytes) in [(1, &f.out), (2, &f.err)] {
        match f.disp[i] {
            Disposition::Pipe => {
                if let Some(p) = &pipes[i - 1] {
                    pipe_output(p, bytes)?;
                }
            }
            Disposition::Inherit => with_host(|h| h.write_out_bytes(bytes, i == 2)),
            Disposition::Ignore => {}
        }
    }
    let code = f
        .status
        .map_or_else(|| null.clone(), |c| Value::Float(c as f64));
    let signal = match f.signal.and_then(signal_name) {
        Some(name) => with_host(|h| h.new_str(name)),
        None => null.clone(),
    };
    set_prop(child, "exitCode", code.clone());
    set_prop(child, "signalCode", signal.clone());
    emit(child, "exit", vec![code.clone(), signal.clone()])?;
    emit(child, "close", vec![code, signal])?;
    for (i, p) in pipes.iter().enumerate() {
        if let (Some(p), Disposition::Pipe) = (p, f.disp[i + 1]) {
            emit(p, "close", Vec::new())?;
        }
    }
    Ok(())
}

/// The name of signal number `n` (`SIGTERM`), from the same table
/// `process.kill` reads.
fn signal_name(n: i32) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "SIGHUP", "SIGINT", "SIGQUIT", "SIGILL", "SIGTRAP", "SIGABRT", "SIGBUS", "SIGFPE",
        "SIGKILL", "SIGUSR1", "SIGSEGV", "SIGUSR2", "SIGPIPE", "SIGALRM", "SIGTERM", "SIGCHLD",
        "SIGCONT", "SIGSTOP", "SIGTSTP", "SIGWINCH",
    ];
    NAMES
        .iter()
        .copied()
        .find(|name| super::process::signal_number(name) == Some(n))
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

    let full_cmd = std::iter::once(file.clone())
        .chain(cmd_args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");

    match run(&file, &cmd_args, &spawn_opts(args, 2)) {
        Ok(r) => {
            let stdout_buf = super::buffer::from_bytes(&r.stdout);
            let stderr_buf = super::buffer::from_bytes(&r.stderr);
            let null = with_host(|h| h.null());
            if let Some(cb) = cb {
                let so = String::from_utf8_lossy(&r.stdout).into_owned();
                let se = String::from_utf8_lossy(&r.stderr).into_owned();
                // As in `exec`, the callback takes an ERROR OBJECT. This built
                // a STRING, so `err instanceof Error` was false and every field
                // a caller reads — `code`, `cmd`, `killed`, `signal` — was
                // `undefined`; the wording was this module's own too. Node's
                // `cmd` here is the file and its arguments joined, since there
                // is no shell command line to quote.
                let err = if r.status == Some(0) {
                    null.clone()
                } else {
                    exec_error(&full_cmd, &r)
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
        // A missing binary is reported THROUGH the callback — `execFile` is
        // async, so it does not throw. Throwing here meant the caller's
        // error-first handler never ran and the whole script died instead.
        Err(e) => {
            let err = spawn_error(&file, &cmd_args, &e);
            if let Some(cb) = cb {
                let (empty1, empty2) = with_host(|h| (h.new_str(""), h.new_str("")));
                with_host(|h| h.queue_micro(cb, vec![err, empty1, empty2]));
                let null = with_host(|h| h.null());
                let mut m = IndexMap::new();
                m.insert("pid".into(), Value::Undef);
                m.insert("exitCode".into(), null.clone());
                m.insert("signalCode".into(), null.clone());
                m.insert("killed".into(), Value::Bool(false));
                m.insert("connected".into(), Value::Bool(false));
                m.insert("stdout".into(), null.clone());
                m.insert("stderr".into(), null);
                return Ok(child_object(m));
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
    if super::events::METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
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
