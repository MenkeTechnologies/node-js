//! Node `cluster` — real process-fork model over `std::process::Command`.
//!
//! # What is real
//!
//! * **`cluster.fork([env])`** spawns a genuine OS child process. It re-launches
//!   the SAME runtime binary (`std::env::current_exe()`) on the SAME entry script
//!   (`process.argv[1]`, i.e. `settings.exec`), with an env marker
//!   (`CLUSTER_WORKER=<id>` + Node's own `NODE_UNIQUE_ID=<id>`) plus any caller
//!   `env` overrides. The child therefore runs the whole program again, but this
//!   time in worker mode. This is the actual Unix master/worker fork model, not a
//!   simulation.
//! * **`cluster.isPrimary`/`isMaster`/`isWorker`** are derived from that env
//!   marker: the primary has neither `CLUSTER_WORKER` nor `NODE_UNIQUE_ID` set; a
//!   forked child has one set, so it reports `isWorker === true`.
//! * **Worker lifecycle events** are real, best-effort: `'fork'` fires
//!   synchronously from `fork()`, `'online'` is posted onto the event loop
//!   immediately after the child launches, and `'exit'` fires when a background
//!   reaper thread observes the child process actually exit (via `Child::wait`).
//!   Each live worker `incr_handle`s the loop so the primary stays alive while
//!   workers run, and `'exit'` `decr_handle`s it.
//! * **`Worker.kill([signal])`** delivers a real signal to the child pid
//!   (`libc::kill`), so `cluster.workers[id].kill()` truly terminates the process.
//! * **`cluster.workers`** maps live worker id → `Worker`, and **`cluster.worker`**
//!   in a forked child is a `Worker` for itself (id from the env marker).
//!
//! # Documented limitations (honest, never a silent fake)
//!
//! * **No primary↔worker IPC channel.** Node connects each fork over a pipe and
//!   ships `worker.send(msg)` / `process.on('message')` across it. node-js does
//!   not wire a cross-process pipe here, so `Worker.send()` is a documented no-op
//!   that returns `false`, and there is no `'message'` delivery between primary
//!   and cluster workers. (In-process message passing exists in
//!   `worker_threads`, which shares one address space; cluster workers are
//!   separate OS processes and would need a real socket/pipe channel.)
//! * **No shared listening socket.** Node's primary opens the listen socket once
//!   and hands the same file descriptor to every worker so N workers accept on ONE
//!   port (SO_REUSEPORT / fd passing). node-js does not pass fds across the fork,
//!   so each worker that calls `server.listen(port)` binds its OWN socket — true
//!   round-robin load balancing across workers on a single port is NOT provided.
//!   Consequently `'listening'` is not emitted (no fd hand-off to observe) and
//!   `Worker.disconnect()` cannot gracefully drain an IPC/socket channel: it marks
//!   the worker disconnected and emits `'disconnect'`, but the real way to stop a
//!   worker is `Worker.kill()`.

use super::arg_str;
use crate::host::{with_host, IoTask, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Callable module members. The EventEmitter surface is included so that
/// `cluster.on('exit', …)` / `cluster.emit(…)` route through `stdlib::call`
/// (`cluster.<method>`) to the process-wide cluster emitter.
pub const METHODS: &[&str] = &[
    "fork",
    "setupPrimary",
    "setupMaster",
    "disconnect",
    // EventEmitter surface (delegated to the cluster emitter).
    "on",
    "addListener",
    "prependListener",
    "once",
    "prependOnceListener",
    "emit",
    "removeListener",
    "off",
    "removeAllListeners",
    "listenerCount",
    "listeners",
    "eventNames",
    "setMaxListeners",
    "getMaxListeners",
];

/// Instance methods on a `Worker` (`@@native` tag `"ClusterWorker"`), beyond the
/// shared EventEmitter surface.
pub const WORKER_METHODS: &[&str] = &[
    "send",
    "kill",
    "destroy",
    "disconnect",
    "isConnected",
    "isDead",
];

/// The shared EventEmitter method names delegated to `events`.
const EMITTER_METHODS: &[&str] = &[
    "on",
    "addListener",
    "prependListener",
    "once",
    "prependOnceListener",
    "emit",
    "removeListener",
    "off",
    "removeAllListeners",
    "listenerCount",
    "listeners",
    "eventNames",
    "setMaxListeners",
    "getMaxListeners",
];

/// Monotonic worker-id source (matches Node: ids start at 1 and count up).
static NEXT_WORKER_ID: AtomicU64 = AtomicU64::new(1);

/// Stored `setupPrimary` settings. `None` fields fall back to the current
/// process's `argv` at fork time (Node's defaults).
#[derive(Default, Clone)]
struct Settings {
    /// The worker entry script (`settings.exec`); defaults to `process.argv[1]`.
    exec: Option<String>,
    /// Args passed to the worker (`settings.args`); defaults to `argv[2..]`.
    args: Option<Vec<String>>,
    /// Runtime flags (`settings.execArgv`); node-js has none meaningful, kept for
    /// surface parity.
    exec_argv: Option<Vec<String>>,
    /// Whether to silence the worker's stdio (`settings.silent`).
    silent: bool,
}

thread_local! {
    /// Live workers owned by the primary, keyed by worker id. Only touched on the
    /// main (primary) thread.
    static WORKERS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
    /// The process-wide `cluster` EventEmitter (cluster IS an emitter in Node);
    /// created lazily and cached so `cluster.on(...)` and the fired lifecycle
    /// events share one object.
    static CLUSTER_EMITTER: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// The cached `cluster.worker` (self) object inside a forked child.
    static SELF_WORKER: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// The stored `setupPrimary` settings.
    static SETTINGS: RefCell<Settings> = RefCell::new(Settings::default());
}

// ── primary/worker detection ─────────────────────────────────────────────────

/// The env marker used to mark a forked child as a cluster worker. Node uses
/// `NODE_UNIQUE_ID`; we honor both it and our explicit `CLUSTER_WORKER`.
fn worker_id_from_env() -> Option<u64> {
    std::env::var("CLUSTER_WORKER")
        .ok()
        .or_else(|| std::env::var("NODE_UNIQUE_ID").ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// True in the primary process (no worker env marker set).
fn is_primary() -> bool {
    worker_id_from_env().is_none()
}

// ── module dispatch ──────────────────────────────────────────────────────────

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // EventEmitter methods route to the process-wide cluster emitter.
    if EMITTER_METHODS.contains(&method) {
        let em = cluster_emitter();
        return Some(super::events::instance_call(&em, method, args.to_vec()));
    }
    Some(match method {
        "fork" => fork(args),
        "setupPrimary" | "setupMaster" => setup_primary(args),
        "disconnect" => disconnect(args),
        _ => return None,
    })
}

/// Non-function members of the `cluster` namespace.
pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "isPrimary" | "isMaster" => Value::Bool(is_primary()),
        "isWorker" => Value::Bool(!is_primary()),
        "workers" => workers_object(),
        "worker" => {
            if is_primary() {
                with_host(|h| h.null())
            } else {
                self_worker()
            }
        }
        "settings" => settings_object(),
        // Node exposes SCHED_RR/SCHED_NONE; node-js does no cross-worker load
        // balancing (see header), so report SCHED_NONE.
        "SCHED_NONE" => Value::Float(1.0),
        "SCHED_RR" => Value::Float(2.0),
        "schedulingPolicy" => Value::Float(1.0),
        _ => return None,
    })
}

// ── fork ─────────────────────────────────────────────────────────────────────

/// `cluster.fork([env])` — spawn a new OS process re-running the entry script in
/// worker mode. Primary-only.
fn fork(args: &[Value]) -> Result<Value, String> {
    if !is_primary() {
        return Err("Error: cluster.fork() can only be called from the primary process".into());
    }

    let s = SETTINGS.with(|s| s.borrow().clone());
    let exec = s
        .exec
        .clone()
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_default();
    if exec.is_empty() {
        return Err(
            "Error: cluster.fork() requires a main script (process.argv[1]); none was found".into(),
        );
    }
    let fwd_args: Vec<String> = s
        .args
        .clone()
        .unwrap_or_else(|| std::env::args().skip(2).collect());
    let exe = std::env::current_exe().map_err(|e| format!("Error: cluster.fork(): {e}"))?;

    // Read caller `env` overrides off the JS heap before touching the OS.
    let overrides = args.first().map(env_overrides).unwrap_or_default();

    let id = NEXT_WORKER_ID.fetch_add(1, Ordering::SeqCst);

    let mut cmd = Command::new(exe);
    cmd.arg(&exec);
    cmd.args(&fwd_args);
    cmd.env("CLUSTER_WORKER", id.to_string());
    cmd.env("NODE_UNIQUE_ID", id.to_string());
    for (k, v) in overrides {
        cmd.env(k, v);
    }
    if s.silent {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    } else {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("Error: cluster.fork(): {e}"))?;
    let pid = child.id();

    // Build + register the Worker, keep the loop alive, then wire events.
    let worker = new_worker(id, pid);
    WORKERS.with(|w| {
        w.borrow_mut().insert(id, worker.clone());
    });
    with_host(|h| h.incr_handle());

    // `'fork'` fires synchronously (Node parity), on the cluster emitter only.
    let _ = emit_on(&cluster_emitter(), "fork", vec![worker.clone()]);

    // `'online'` is best-effort: posted onto the loop right after launch (no IPC
    // "online" handshake exists — see header).
    let io_online = with_host(|h| h.io_sender());
    let _ = io_online.send(Box::new(move || dispatch_online(id)));

    // Reaper: wait for the real child exit on a background thread, then post the
    // `'exit'` event onto the main loop.
    let io_exit: std::sync::mpsc::Sender<IoTask> = with_host(|h| h.io_sender());
    std::thread::spawn(move || {
        let mut child = child;
        let code = child.wait().ok().and_then(|st| st.code()).unwrap_or(0);
        let _ = io_exit.send(Box::new(move || dispatch_exit(id, code)));
    });

    Ok(worker)
}

/// Read a caller `env` object into `(key, value)` string pairs (skipping the
/// hidden `@@`-prefixed internal keys).
fn env_overrides(v: &Value) -> Vec<(String, String)> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| !k.starts_with("@@"))
            .map(|(k, val)| (k.clone(), h.str_of(val)))
            .collect(),
        _ => Vec::new(),
    })
}

// ── setupPrimary / settings ──────────────────────────────────────────────────

/// `cluster.setupPrimary(opts)` (alias `setupMaster`) — merge `opts` into the
/// stored settings. Returns `undefined`.
fn setup_primary(args: &[Value]) -> Result<Value, String> {
    if let Some(opts) = args.first() {
        let exec = str_prop(opts, "exec");
        let arr = arr_prop(opts, "args");
        let ea = arr_prop(opts, "execArgv");
        let silent = bool_prop(opts, "silent");
        SETTINGS.with(|s| {
            let mut s = s.borrow_mut();
            if exec.is_some() {
                s.exec = exec;
            }
            if arr.is_some() {
                s.args = arr;
            }
            if ea.is_some() {
                s.exec_argv = ea;
            }
            if let Some(b) = silent {
                s.silent = b;
            }
        });
    }
    Ok(Value::Undef)
}

/// `cluster.settings` — the effective settings object (stored values, with
/// `argv` fallbacks resolved like Node).
fn settings_object() -> Value {
    let s = SETTINGS.with(|s| s.borrow().clone());
    let exec = s
        .exec
        .clone()
        .unwrap_or_else(|| std::env::args().nth(1).unwrap_or_default());
    let args_vec = s
        .args
        .clone()
        .unwrap_or_else(|| std::env::args().skip(2).collect());
    let exec_argv = s.exec_argv.clone().unwrap_or_default();
    with_host(|h| {
        let arg_items: Vec<Value> = args_vec.into_iter().map(|a| h.new_str(a)).collect();
        let args_arr = h.new_array(arg_items);
        let ea_items: Vec<Value> = exec_argv.into_iter().map(|a| h.new_str(a)).collect();
        let ea_arr = h.new_array(ea_items);
        let exec_v = h.new_str(exec);
        let mut m = IndexMap::new();
        m.insert("exec".into(), exec_v);
        m.insert("args".into(), args_arr);
        m.insert("execArgv".into(), ea_arr);
        m.insert("silent".into(), Value::Bool(s.silent));
        h.new_object(m)
    })
}

// ── disconnect (module-level) ────────────────────────────────────────────────

/// `cluster.disconnect([callback])` — mark every live worker disconnected and
/// emit `'disconnect'`. Without an IPC channel this cannot gracefully drain a
/// worker; `Worker.kill()` is the real termination path (see header). If a
/// callback is supplied it is invoked once, synchronously, after signalling.
fn disconnect(args: &[Value]) -> Result<Value, String> {
    let workers: Vec<Value> = WORKERS.with(|w| w.borrow().values().cloned().collect());
    for wk in workers {
        mark_disconnected(&wk);
    }
    if let Some(cb) = args.first() {
        if with_host(|h| h.type_of(cb)) == "function" {
            crate::host::invoke(cb, vec![], None)?;
        }
    }
    Ok(Value::Undef)
}

// ── Worker instances (`@@native` tag "ClusterWorker") ────────────────────────

/// Build a `Worker` emitter object carrying `.id`, `.process` (`{ pid }`) and the
/// hidden bookkeeping props.
fn new_worker(id: u64, pid: u32) -> Value {
    let proc_obj = with_host(|h| {
        let mut p = IndexMap::new();
        p.insert("pid".into(), Value::Float(pid as f64));
        p.insert("connected".into(), Value::Bool(true));
        h.new_object(p)
    });
    let mut extra = IndexMap::new();
    extra.insert("id".into(), Value::Float(id as f64));
    extra.insert("process".into(), proc_obj);
    extra.insert("@@cwid".into(), Value::Float(id as f64));
    extra.insert("@@pid".into(), Value::Float(pid as f64));
    extra.insert("@@connected".into(), Value::Bool(true));
    super::net::new_emitter_object("ClusterWorker", extra)
}

/// The forked child's own `Worker` (`cluster.worker`), created once and cached.
fn self_worker() -> Value {
    if let Some(v) = SELF_WORKER.with(|c| c.borrow().clone()) {
        return v;
    }
    let id = worker_id_from_env().unwrap_or(0);
    let w = new_worker(id, std::process::id());
    SELF_WORKER.with(|c| *c.borrow_mut() = Some(w.clone()));
    w
}

/// `cluster.workers` — an object mapping id (string key) → live `Worker`.
fn workers_object() -> Value {
    let entries: Vec<(String, Value)> = WORKERS.with(|w| {
        w.borrow()
            .iter()
            .map(|(id, wk)| (id.to_string(), wk.clone()))
            .collect()
    });
    with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in entries {
            m.insert(k, v);
        }
        h.new_object(m)
    })
}

pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if EMITTER_METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        // No cross-process IPC channel exists (see header): `send` is a documented
        // no-op returning `false` (Node returns a boolean write-queued flag).
        "send" => Ok(Value::Bool(false)),
        "kill" | "destroy" => {
            let sig = signal_number(args.first());
            if let Some(pid) = pid_of(recv) {
                // SAFETY: `kill` is a plain syscall on a pid + signal number.
                unsafe {
                    libc::kill(pid as libc::pid_t, sig);
                }
            }
            mark_disconnected(recv);
            Ok(Value::Undef)
        }
        // Best-effort: mark disconnected + emit `'disconnect'`; cannot drain an IPC
        // channel (none exists).
        "disconnect" => {
            mark_disconnected(recv);
            Ok(recv.clone())
        }
        "isConnected" => Ok(Value::Bool(bool_prop(recv, "@@connected").unwrap_or(false))),
        "isDead" => {
            let id = pid_or_id(recv, "@@cwid");
            let alive = id
                .map(|i| WORKERS.with(|w| w.borrow().contains_key(&i)))
                .unwrap_or(false);
            Ok(Value::Bool(!alive))
        }
        _ => Err(crate::host::type_error(&format!(
            "worker.{method} is not a function"
        ))),
    }
}

/// Mark a worker `@@connected = false` (and its `process.connected`), then emit
/// `'disconnect'` on the worker and on the cluster emitter.
fn mark_disconnected(worker: &Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(worker) {
            p.insert("@@connected".into(), Value::Bool(false));
        }
    });
    let proc = with_host(|h| match h.get(worker) {
        Some(JsObj::Object(p)) => p.get("process").cloned(),
        _ => None,
    });
    if let Some(proc) = proc {
        with_host(|h| {
            if let Some(JsObj::Object(p)) = h.get_mut(&proc) {
                p.insert("connected".into(), Value::Bool(false));
            }
        });
    }
    let _ = emit_on(worker, "disconnect", vec![]);
    let _ = emit_on(&cluster_emitter(), "disconnect", vec![worker.clone()]);
}

// ── event delivery (run on the main loop) ────────────────────────────────────

/// Fire `'online'` on the worker and the cluster emitter for `id`.
fn dispatch_online(id: u64) -> Result<(), String> {
    let Some(worker) = WORKERS.with(|w| w.borrow().get(&id).cloned()) else {
        return Ok(());
    };
    emit_on(&worker, "online", vec![])?;
    emit_on(&cluster_emitter(), "online", vec![worker])
}

/// Fire `'exit'` for `id` (worker: `(code, signal)`; cluster: `(worker, code,
/// signal)`), drop it from the registry, and release the loop handle.
fn dispatch_exit(id: u64, code: i32) -> Result<(), String> {
    let Some(worker) = WORKERS.with(|w| w.borrow().get(&id).cloned()) else {
        return Ok(());
    };
    let null_sig = with_host(|h| h.null());
    emit_on(
        &worker,
        "exit",
        vec![Value::Float(code as f64), null_sig.clone()],
    )?;
    emit_on(
        &cluster_emitter(),
        "exit",
        vec![worker, Value::Float(code as f64), null_sig],
    )?;
    WORKERS.with(|w| {
        w.borrow_mut().remove(&id);
    });
    with_host(|h| h.decr_handle());
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// The process-wide `cluster` EventEmitter, created once and cached.
fn cluster_emitter() -> Value {
    if let Some(v) = CLUSTER_EMITTER.with(|c| c.borrow().clone()) {
        return v;
    }
    let e = super::events::new_emitter();
    CLUSTER_EMITTER.with(|c| *c.borrow_mut() = Some(e.clone()));
    e
}

/// Emit `name` (with `args`) on `emitter`, releasing the host borrow before
/// dispatch (listeners re-enter the host).
fn emit_on(emitter: &Value, name: &str, mut args: Vec<Value>) -> Result<(), String> {
    let mut a = vec![with_host(|h| h.new_str(name))];
    a.append(&mut args);
    super::events::instance_call(emitter, "emit", a).map(|_| ())
}

/// The child pid recorded on a `Worker` (`@@pid`).
fn pid_of(worker: &Value) -> Option<u32> {
    pid_or_id(worker, "@@pid").map(|n| n as u32)
}

/// Read a numeric hidden prop off a `Worker`.
fn pid_or_id(worker: &Value, key: &str) -> Option<u64> {
    with_host(|h| match h.get(worker) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.to_number(v) as u64),
        _ => None,
    })
}

/// Map a kill signal argument (a name like `"SIGKILL"` or a number) to its signal
/// number; defaults to `SIGTERM`.
fn signal_number(arg: Option<&Value>) -> libc::c_int {
    let Some(v) = arg else { return libc::SIGTERM };
    let n = with_host(|h| h.to_number(v));
    if n.is_finite() && n != 0.0 {
        return n as libc::c_int;
    }
    match arg_str(std::slice::from_ref(v), 0).to_uppercase().as_str() {
        "SIGKILL" => libc::SIGKILL,
        "SIGINT" => libc::SIGINT,
        "SIGHUP" => libc::SIGHUP,
        "SIGQUIT" => libc::SIGQUIT,
        "SIGUSR1" => libc::SIGUSR1,
        "SIGUSR2" => libc::SIGUSR2,
        _ => libc::SIGTERM,
    }
}

/// Read a string property off an options object (`None` if absent/empty).
fn str_prop(obj: &Value, key: &str) -> Option<String> {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p
            .get(key)
            .map(|v| h.str_of(v))
            .filter(|s| !s.is_empty() && s != "undefined"),
        _ => None,
    })
}

/// Read a boolean property off an options object.
fn bool_prop(obj: &Value, key: &str) -> Option<bool> {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.truthy(v)),
        _ => None,
    })
}

/// Read an array-of-strings property off an options object.
fn arr_prop(obj: &Value, key: &str) -> Option<Vec<String>> {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => match p.get(key).and_then(|v| h.get(v)) {
            Some(JsObj::Array(items)) => Some(items.iter().map(|v| h.str_of(v)).collect()),
            _ => None,
        },
        _ => None,
    })
}
