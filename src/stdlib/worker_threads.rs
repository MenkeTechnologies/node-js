//! Node `worker_threads`: real OS-thread workers with fully isolated heaps.
//!
//! # Model (matches Node: workers do NOT share the JS heap)
//!
//! node-js's entire runtime — the `JsHost` heap, the module cache, the event
//! loop channel — lives in `thread_local!`s (`host::HOST`, `module`'s statics).
//! So spawning a fresh OS thread automatically gives that thread its OWN
//! isolated interpreter and heap. A `Worker` here is therefore a real
//! `std::thread` that calls `crate::eval_file`/`eval_str` on the worker file,
//! running it against that thread's own clean `thread_local` host. Nothing on
//! the JS heap is shared between the main thread and a worker, or between two
//! workers — exactly Node's isolation guarantee.
//!
//! # Why messages cross as JSON strings, never `Value`s
//!
//! `fusevm::Value` is a per-thread heap handle (`Value::Obj(u32)` indexes the
//! calling thread's `JsHost.heap`); it is neither `Send` nor meaningful on
//! another thread. So a message can NEVER be a `Value`. Every message crosses
//! the thread boundary as a plain `String` of JSON:
//!
//! ```text
//!   sender thread:   value ─JSON.stringify([value])→ String  (on sender's heap)
//!   channel:         String  (Send)
//!   receiver thread: String ─JSON.parse(s)[0]→ value        (on receiver's heap)
//! ```
//!
//! The value is wrapped in a one-element array before `JSON.stringify` so that
//! top-level primitives AND `undefined` round-trip through a single always-valid
//! JSON document (`JSON.stringify(undefined)` is itself `undefined`, not a
//! string — the array wrapper avoids that). Deserialization unwraps `[0]` on the
//! receiving thread's own heap.
//!
//! ## Serialization is a JSON subset of structured clone (documented limitation)
//!
//! Only JSON-serializable data transfers: objects, arrays, strings, numbers,
//! booleans, null. `undefined` becomes `null` (JSON semantics), and functions,
//! symbols, `Map`/`Set`, cycles, `BigInt`, and `ArrayBuffer` transfers are NOT
//! supported (a `BigInt` makes `JSON.stringify` throw, surfaced as a thrown
//! error from `postMessage`, matching Node's `DataCloneError` in spirit). This
//! is an honest subset, never a silent fake.
//!
//! # Bidirectional message flow (both directions are real)
//!
//! * worker → main (`parentPort.postMessage`): the worker serializes on its own
//!   heap and posts an `IoTask` onto the MAIN loop's `io_sender` (captured at
//!   construction). The task runs on the main thread, deserializes into a fresh
//!   `Value` on the main heap, and emits `'message'` on the `Worker` object.
//! * main → worker (`worker.postMessage`): the main thread serializes and sends
//!   the JSON string over an `mpsc` channel to the worker. A per-worker "bridge"
//!   thread (started when the worker adds a `parentPort` `'message'` listener)
//!   forwards each string as an `IoTask` onto the WORKER loop's `io_sender`; the
//!   task runs on the worker thread, deserializes on the worker heap, and emits
//!   `'message'` on `parentPort`. The bridge is required because the worker's
//!   event loop (`host::run_event_loop`) blocks only on its own I/O channel and
//!   this module cannot modify it — same pattern `net` uses for socket reads.
//!
//! # Liveness
//!
//! `new Worker` `incr_handle`s the MAIN loop so the process stays alive while the
//! worker runs; the worker's `'exit'` `decr_handle`s it. On the worker side,
//! registering a `parentPort` `'message'` listener `incr_handle`s the WORKER loop
//! (keeping the worker alive to receive messages), and `terminate` releases it.
//!
//! # terminate is cooperative (documented limitation)
//!
//! Rust has no safe thread cancellation, so `terminate` signals the worker (via
//! the bridge) to `decr_handle` and let its event loop unwind; a worker parked in
//! its message loop exits promptly. A worker spinning in a tight *synchronous* JS
//! loop is not force-killed — there is no safe preemption point. `terminate`
//! returns `undefined` (awaiting it resolves to `undefined`).

use crate::host::{with_host, IoTask, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

/// Module-level `worker_threads` functions. NOTE: these route only if the parent
/// adds a `"worker_threads"` arm to `stdlib::is_method` and `stdlib::call` (the
/// module previously had no callable methods) — see the report.
pub const METHODS: &[&str] = &[
    "getEnvironmentData",
    "setEnvironmentData",
    "receiveMessageOnPort",
    "markAsUntransferable",
    "isMarkedAsUntransferable",
    "markAsUncloneable",
    "moveMessagePortToContext",
];

/// Instance methods on a `BroadcastChannel` object.
pub const BROADCAST_CHANNEL_METHODS: &[&str] = &[
    "postMessage",
    "close",
    "ref",
    "unref",
    "addEventListener",
    "removeEventListener",
];

/// Instance methods on a `Worker` (main-side handle), beyond the shared
/// EventEmitter surface (`on`/`once`/`emit`/…).
pub const WORKER_METHODS: &[&str] = &["postMessage", "terminate", "ref", "unref"];

/// Instance methods on a `MessagePort` (the worker-side `parentPort`), beyond the
/// shared EventEmitter surface.
pub const PORT_METHODS: &[&str] = &["postMessage", "close", "start", "ref", "unref"];

/// Global, cross-thread thread-id source. The main thread is id 0; each `Worker`
/// (including nested workers spawned from a worker) gets a fresh positive id.
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

/// A message crossing the main→worker `mpsc` channel. Both variants are `Send`
/// (a JSON `String` / unit) — never a `Value`.
enum WorkerMsg {
    Data(String),
    Terminate,
}

/// An event to raise on a `Worker` object, produced by the worker thread and run
/// as an `IoTask` on the MAIN thread. All fields are `Send` plain data.
enum MainEvent {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
}

/// Main-thread registry entry for a live worker (keyed by thread id).
struct WorkerRec {
    /// The `Worker` EventEmitter object (lives on the main heap).
    emitter: Value,
    /// Sends main→worker messages / the terminate signal.
    to_worker: Sender<WorkerMsg>,
}

thread_local! {
    /// Live workers owned by THIS thread (the main thread, or a worker that
    /// itself spawned sub-workers). Only ever touched on the owning thread.
    static WORKERS: RefCell<HashMap<u64, WorkerRec>> = RefCell::new(HashMap::new());
}

/// Per-worker-thread context, set once when the worker thread starts, read while
/// its file runs. Absent (`None`) on the main thread — that is how `isMainThread`
/// is computed.
struct WorkerCtx {
    thread_id: u64,
    /// `workerData` serialized as JSON on the spawning (main) thread; deserialized
    /// lazily on this thread when `workerData` is read.
    worker_data_json: String,
    /// The MAIN loop's `io_sender`, to post worker→main events.
    main_tx: Sender<IoTask>,
    /// This worker's id (registry key on the main side).
    self_id: u64,
    /// Receives main→worker messages; taken out when the bridge thread starts.
    rx: Option<Receiver<WorkerMsg>>,
    /// Whether the forwarding bridge thread has been started.
    bridge_started: bool,
}

thread_local! {
    static WORKER_CTX: RefCell<Option<WorkerCtx>> = const { RefCell::new(None) };
    /// The worker-side `parentPort` object, created lazily on first access and
    /// cached (so both the JS file and the delivery `IoTask` share one object).
    static PARENT_PORT: RefCell<Option<Value>> = const { RefCell::new(None) };

    // ── MessageChannel state (both ports live on ONE thread) ──────────────────
    /// port id → the `MessagePort` object.
    static CHANNEL_PORTS: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
    /// port id → its peer's port id (posting to one enqueues on the other).
    static CH_PEER: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
    /// port id → JSON messages queued FOR that port (drained by
    /// `receiveMessageOnPort` or async `'message'` delivery once started).
    static CH_QUEUE: RefCell<HashMap<u64, VecDeque<String>>> = RefCell::new(HashMap::new());
    /// port ids whose async `'message'` delivery is active (a listener was added
    /// or `start()` was called).
    static CH_STARTED: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());

    // ── BroadcastChannel state (in-process, SAME-thread only) ─────────────────
    /// channel name → the live `BroadcastChannel` objects on this thread.
    static BCAST: RefCell<HashMap<String, Vec<(u64, Value)>>> = RefCell::new(HashMap::new());

    // ── markAsUntransferable / markAsUncloneable flags (heap ids) ─────────────
    static UNTRANSFERABLE: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static UNCLONEABLE: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

/// Process-global `environmentData` map (shared across threads, so a value set on
/// the main thread is visible via `getEnvironmentData` on a worker). Values cross
/// as JSON strings because `fusevm::Value` is not `Send`.
fn env_data() -> &'static Mutex<HashMap<String, String>> {
    static ENV_DATA: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    ENV_DATA.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A fresh unique port / broadcast-channel id (shares the thread-id counter's
/// space is unnecessary — its own counter keeps ids distinct within a thread).
static NEXT_PORT_ID: AtomicU64 = AtomicU64::new(1);

// ── serialization (JSON subset of structured clone) ──────────────────────────

/// Serialize a value to a JSON string on the CURRENT thread's heap. Wrapped in a
/// one-element array so primitives and `undefined` round-trip through one always-
/// valid JSON document. Errors (e.g. a `BigInt`) propagate as a thrown error.
fn serialize(v: &Value) -> Result<String, String> {
    let arr = with_host(|h| h.new_array(vec![v.clone()]));
    let json = crate::builtins::call_builtin_function("JSON.stringify", vec![arr])?;
    Ok(with_host(|h| h.str_of(&json)))
}

/// Deserialize a JSON string produced by `serialize` into a fresh value on the
/// CURRENT thread's heap (unwrapping the array wrapper).
fn deserialize(json: &str) -> Result<Value, String> {
    let sv = with_host(|h| h.new_str(json.to_string()));
    let arr = crate::builtins::call_builtin_function("JSON.parse", vec![sv])?;
    Ok(with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => items.first().cloned().unwrap_or(Value::Undef),
        _ => Value::Undef,
    }))
}

// ── small heap helpers ───────────────────────────────────────────────────────

fn arg0(args: &[Value]) -> Value {
    args.first().cloned().unwrap_or(Value::Undef)
}

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn u64_prop(recv: &Value, key: &str) -> Option<u64> {
    get_prop(recv, key).map(|v| with_host(|h| h.to_number(&v)) as u64)
}

/// Emit `name` (with `args`) on an emitter object, releasing the host borrow
/// before dispatch (listeners re-enter the host).
fn emit_event(emitter: &Value, name: &str, mut args: Vec<Value>) -> Result<(), String> {
    let mut a = vec![with_host(|h| h.new_str(name))];
    a.append(&mut args);
    super::events::instance_call(emitter, "emit", a).map(|_| ())
}

// ── thread-context queries (drive the module constants) ──────────────────────

fn is_worker_thread() -> bool {
    WORKER_CTX.with(|c| c.borrow().is_some())
}

fn current_thread_id() -> u64 {
    WORKER_CTX.with(|c| c.borrow().as_ref().map(|x| x.thread_id).unwrap_or(0))
}

/// `workerData` for this thread: the deserialized options payload on a worker,
/// `undefined` on the main thread.
fn current_worker_data() -> Value {
    let json = WORKER_CTX.with(|c| c.borrow().as_ref().map(|x| x.worker_data_json.clone()));
    match json {
        Some(j) => deserialize(&j).unwrap_or(Value::Undef),
        None => Value::Undef,
    }
}

/// The worker-side `parentPort` (a `MessagePort` emitter), created once per worker
/// thread and cached. Only meaningful on a worker thread.
fn ensure_parent_port() -> Value {
    if let Some(p) = PARENT_PORT.with(|p| p.borrow().clone()) {
        return p;
    }
    let port = super::net::new_emitter_object("MessagePort", IndexMap::new());
    PARENT_PORT.with(|p| *p.borrow_mut() = Some(port.clone()));
    port
}

// ── module constants (reached via `stdlib::constant("worker_threads", name)`) ─

/// Non-function members of the `worker_threads` namespace:
/// `isMainThread`/`threadId`/`parentPort`/`workerData`, and the `Worker`
/// constructor (as a `Builtin("Worker")` so `new Worker(...)` reaches
/// `construct_worker`).
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "isMainThread" => Some(Value::Bool(!is_worker_thread())),
        "threadId" => Some(Value::Float(current_thread_id() as f64)),
        "parentPort" => Some(if is_worker_thread() {
            ensure_parent_port()
        } else {
            // On the main thread `parentPort` is `null` (Node parity).
            with_host(|h| h.null())
        }),
        "workerData" => Some(current_worker_data()),
        "Worker" | "MessageChannel" | "BroadcastChannel" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        _ => None,
    }
}

/// Module-level dispatch. Routes only if the parent adds a `"worker_threads"` arm
/// to `stdlib::call` (see the report).
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "setEnvironmentData" => {
            let key = super::arg_str(args, 0);
            match args.get(1) {
                // An `undefined` value deletes the key (Node behavior).
                Some(v) if !matches!(v, Value::Undef) => match serialize(v) {
                    Ok(json) => {
                        if let Ok(mut m) = env_data().lock() {
                            m.insert(key, json);
                        }
                        Ok(Value::Undef)
                    }
                    Err(e) => Err(e),
                },
                _ => {
                    if let Ok(mut m) = env_data().lock() {
                        m.remove(&key);
                    }
                    Ok(Value::Undef)
                }
            }
        }
        "getEnvironmentData" => {
            let key = super::arg_str(args, 0);
            let json = env_data().lock().ok().and_then(|m| m.get(&key).cloned());
            match json {
                Some(j) => deserialize(&j),
                None => Ok(Value::Undef),
            }
        }
        "receiveMessageOnPort" => Ok(receive_message_on_port(args.first())),
        // Best-effort transfer/clone flags: node-js clones every message as a JSON
        // subset (see module docs), so these flags do not alter serialization —
        // they are recorded and reflected by the `is*` query for API fidelity.
        "markAsUntransferable" => {
            if let Some(Value::Obj(id)) = args.first() {
                UNTRANSFERABLE.with(|s| s.borrow_mut().insert(*id));
            }
            Ok(Value::Undef)
        }
        "isMarkedAsUntransferable" => Ok(Value::Bool(matches!(
            args.first(),
            Some(Value::Obj(id)) if UNTRANSFERABLE.with(|s| s.borrow().contains(id))
        ))),
        "markAsUncloneable" => {
            if let Some(Value::Obj(id)) = args.first() {
                UNCLONEABLE.with(|s| s.borrow_mut().insert(*id));
            }
            Ok(Value::Undef)
        }
        // node-js has one context, so there is nothing to move the port into;
        // return the port unchanged.
        "moveMessagePortToContext" => Ok(arg0(args)),
        _ => return None,
    })
}

// ── construction: `new Worker(filename[, options])` ──────────────────────────

/// Build a `Worker` and spawn its OS thread (runs on the MAIN thread). The worker
/// thread runs `filename` (a file path, or the code itself when `options.eval` is
/// truthy) against its own fresh `thread_local` host.
pub fn construct_worker(args: &[Value]) -> Result<Value, String> {
    let filename = with_host(|h| h.str_of(&arg0(args)));
    let opts = args.get(1).cloned();
    let is_eval = opts
        .as_ref()
        .and_then(|o| get_prop(o, "eval"))
        .map(|v| with_host(|h| h.truthy(&v)))
        .unwrap_or(false);
    // Serialize workerData NOW, on the main heap, into a Send JSON string.
    let worker_data_json = match opts.as_ref().and_then(|o| get_prop(o, "workerData")) {
        Some(v) => serialize(&v)?,
        None => serialize(&Value::Undef)?, // "[null]"
    };

    let id = NEXT_THREAD_ID.fetch_add(1, Ordering::SeqCst);
    let (to_worker_tx, to_worker_rx) = std::sync::mpsc::channel::<WorkerMsg>();
    let main_tx = with_host(|h| h.io_sender());

    let mut extra = IndexMap::new();
    extra.insert("@@wtid".into(), Value::Float(id as f64));
    extra.insert("threadId".into(), Value::Float(id as f64));
    let emitter = super::net::new_emitter_object("Worker", extra);

    WORKERS.with(|w| {
        w.borrow_mut().insert(
            id,
            WorkerRec {
                emitter: emitter.clone(),
                to_worker: to_worker_tx,
            },
        );
    });
    // Keep the MAIN loop alive while the worker runs.
    with_host(|h| h.incr_handle());

    let spawn_tx = main_tx.clone();
    std::thread::spawn(move || {
        worker_thread_main(
            id,
            filename,
            is_eval,
            worker_data_json,
            spawn_tx,
            to_worker_rx,
        );
    });

    Ok(emitter)
}

/// The worker thread's entry point. Runs on a brand-new OS thread whose
/// `thread_local` host/module state is clean and isolated.
fn worker_thread_main(
    id: u64,
    filename: String,
    is_eval: bool,
    worker_data_json: String,
    main_tx: Sender<IoTask>,
    rx: Receiver<WorkerMsg>,
) {
    WORKER_CTX.with(|c| {
        *c.borrow_mut() = Some(WorkerCtx {
            thread_id: id,
            worker_data_json,
            main_tx: main_tx.clone(),
            self_id: id,
            rx: Some(rx),
            bridge_started: false,
        });
    });

    // `'online'` fires once the worker thread has begun executing.
    post_to_main(&main_tx, id, MainEvent::Online);

    // Run the worker's code on this thread's own isolated host. `eval_file` /
    // `eval_str` call `reset_host()` (fresh heap) and drain the worker's event
    // loop — which, if the worker added a `parentPort` 'message' listener, stays
    // alive processing bridged messages until `terminate`.
    let outcome = if is_eval {
        crate::eval_str(&filename)
    } else {
        crate::eval_file(&filename)
    };

    match outcome {
        Ok(_) => post_to_main(&main_tx, id, MainEvent::Exit(0)),
        Err(e) => {
            post_to_main(&main_tx, id, MainEvent::Error(e));
            post_to_main(&main_tx, id, MainEvent::Exit(1));
        }
    }
}

/// Post a worker→main event as an `IoTask` onto the main loop. The closure is
/// `Send` (captures only `id` + plain data) and runs `dispatch_main` on the main
/// thread, where the `Worker` object and main heap live.
fn post_to_main(main_tx: &Sender<IoTask>, id: u64, ev: MainEvent) {
    let _ = main_tx.send(Box::new(move || dispatch_main(id, ev)));
}

/// Run a worker→main event on the MAIN thread.
fn dispatch_main(id: u64, ev: MainEvent) -> Result<(), String> {
    let emitter = WORKERS.with(|w| w.borrow().get(&id).map(|r| r.emitter.clone()));
    let Some(emitter) = emitter else {
        return Ok(());
    };
    match ev {
        MainEvent::Online => emit_event(&emitter, "online", vec![]),
        MainEvent::Message(json) => {
            let v = deserialize(&json)?;
            emit_event(&emitter, "message", vec![v])
        }
        MainEvent::Error(msg) => {
            let err =
                crate::builtins::construct_builtin("Error", vec![with_host(|h| h.new_str(msg))])?;
            emit_event(&emitter, "error", vec![err])
        }
        MainEvent::Exit(code) => {
            emit_event(&emitter, "exit", vec![Value::Float(code as f64)])?;
            WORKERS.with(|w| {
                w.borrow_mut().remove(&id);
            });
            with_host(|h| h.decr_handle());
            // Wake the loop so a now-idle process can exit.
            let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
            Ok(())
        }
    }
}

// ── worker-side bridge: main→worker message delivery ─────────────────────────

/// Start the forwarding thread that drains the main→worker `mpsc` channel and
/// posts each message as an `IoTask` onto THIS worker's event loop. Idempotent;
/// called when the worker adds a `parentPort` 'message' listener (or `start()`s
/// the port). Also `incr_handle`s the worker loop so it stays alive to receive.
fn start_parent_bridge() {
    // Take the receiver + capture the worker loop's sender while we hold the ctx.
    let worker_io: Option<Sender<IoTask>> = WORKER_CTX.with(|c| {
        let mut cb = c.borrow_mut();
        let ctx = cb.as_mut()?;
        if ctx.bridge_started {
            return None;
        }
        let rx = ctx.rx.take()?;
        ctx.bridge_started = true;
        let io = with_host(|h| h.io_sender());
        with_host(|h| h.incr_handle());
        // Spawn the forwarder; it owns `rx` and a clone of the worker io sender.
        let io_for_thread = io.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    WorkerMsg::Data(json) => {
                        let _ = io_for_thread.send(Box::new(move || parent_deliver(json)));
                    }
                    WorkerMsg::Terminate => {
                        // Release the worker loop so it can unwind and exit.
                        let _ = io_for_thread.send(Box::new(|| {
                            with_host(|h| h.decr_handle());
                            Ok(())
                        }));
                        break;
                    }
                }
            }
        });
        Some(io)
    });
    let _ = worker_io;
}

/// Deliver a main→worker message on the WORKER thread (runs inside the worker
/// event loop): deserialize on the worker heap and emit `'message'` on
/// `parentPort`.
fn parent_deliver(json: String) -> Result<(), String> {
    let port = ensure_parent_port();
    let v = deserialize(&json)?;
    emit_event(&port, "message", vec![v])
}

// ── instance dispatch (from `stdlib::instance_call`) ─────────────────────────

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

pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match tag {
        "Worker" => worker_call(recv, method, args),
        "MessagePort" => port_call(recv, method, args),
        "BroadcastChannel" => broadcast_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// Methods on the main-side `Worker` handle.
fn worker_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if EMITTER_METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        "postMessage" => {
            let json = serialize(&arg0(&args))?;
            if let Some(id) = u64_prop(recv, "@@wtid") {
                WORKERS.with(|w| {
                    if let Some(r) = w.borrow().get(&id) {
                        let _ = r.to_worker.send(WorkerMsg::Data(json));
                    }
                });
            }
            Ok(Value::Undef)
        }
        "terminate" => {
            // Signal the worker; its normal completion emits `'exit'`. Cooperative
            // (see module docs) — returns `undefined`.
            if let Some(id) = u64_prop(recv, "@@wtid") {
                WORKERS.with(|w| {
                    if let Some(r) = w.borrow().get(&id) {
                        let _ = r.to_worker.send(WorkerMsg::Terminate);
                    }
                });
            }
            Ok(Value::Undef)
        }
        "ref" | "unref" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "worker.{method} is not a function"
        ))),
    }
}

/// Methods on the worker-side `parentPort` (`MessagePort`).
fn port_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    // A `MessageChannel` port carries a `@@portid` and is handled separately from
    // the worker `parentPort` (which posts to the spawning thread).
    if let Some(pid) = u64_prop(recv, "@@portid") {
        return channel_port_call(recv, pid, method, args);
    }
    if EMITTER_METHODS.contains(&method) {
        let r = super::events::instance_call(recv, method, args.clone());
        // Adding a 'message' listener implicitly starts message delivery.
        if matches!(
            method,
            "on" | "addListener" | "prependListener" | "once" | "prependOnceListener"
        ) {
            let ev = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            if ev == "message" {
                start_parent_bridge();
            }
        }
        return r;
    }
    match method {
        "postMessage" => {
            let json = serialize(&arg0(&args))?;
            WORKER_CTX.with(|c| {
                if let Some(ctx) = c.borrow().as_ref() {
                    post_to_main(&ctx.main_tx, ctx.self_id, MainEvent::Message(json));
                }
            });
            Ok(Value::Undef)
        }
        "start" => {
            start_parent_bridge();
            Ok(Value::Undef)
        }
        "close" | "ref" | "unref" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "port.{method} is not a function"
        ))),
    }
}

// ── MessageChannel (a pair of linked in-process ports) ───────────────────────

/// `new MessageChannel()` → `{ port1, port2 }`, two `MessagePort` objects linked
/// so that `port1.postMessage(v)` is delivered to `port2` (and vice versa). Both
/// ports live on the calling thread and share its heap; messages are cloned
/// through the JSON subset (`serialize`/`deserialize`) so a posted object is a
/// copy, not a shared reference — matching structured clone's copy semantics.
/// Requires the parent to wire `MessageChannel` construction (see the report).
pub fn construct_message_channel(_args: &[Value]) -> Result<Value, String> {
    let id1 = NEXT_PORT_ID.fetch_add(1, Ordering::SeqCst);
    let id2 = NEXT_PORT_ID.fetch_add(1, Ordering::SeqCst);
    let mut e1 = IndexMap::new();
    e1.insert("@@portid".into(), Value::Float(id1 as f64));
    let port1 = super::net::new_emitter_object("MessagePort", e1);
    let mut e2 = IndexMap::new();
    e2.insert("@@portid".into(), Value::Float(id2 as f64));
    let port2 = super::net::new_emitter_object("MessagePort", e2);

    CHANNEL_PORTS.with(|m| {
        let mut m = m.borrow_mut();
        m.insert(id1, port1.clone());
        m.insert(id2, port2.clone());
    });
    CH_PEER.with(|m| {
        let mut m = m.borrow_mut();
        m.insert(id1, id2);
        m.insert(id2, id1);
    });

    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("port1".into(), port1);
        m.insert("port2".into(), port2);
        h.new_object(m)
    }))
}

/// Method dispatch for a `MessageChannel` port (identified by its `@@portid`).
fn channel_port_call(
    recv: &Value,
    pid: u64,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    if EMITTER_METHODS.contains(&method) {
        let r = super::events::instance_call(recv, method, args.clone());
        // Adding a 'message' listener starts async delivery for this port.
        if matches!(
            method,
            "on" | "addListener" | "prependListener" | "once" | "prependOnceListener"
        ) {
            let ev = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            if ev == "message" {
                start_channel_port(pid);
            }
        }
        return r;
    }
    match method {
        "postMessage" => {
            let json = serialize(&arg0(&args))?;
            channel_post(pid, json);
            Ok(Value::Undef)
        }
        "start" => {
            start_channel_port(pid);
            Ok(Value::Undef)
        }
        "close" => {
            CH_STARTED.with(|s| {
                s.borrow_mut().remove(&pid);
            });
            Ok(Value::Undef)
        }
        "ref" | "unref" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "port.{method} is not a function"
        ))),
    }
}

/// Enqueue a JSON message on `from`'s peer, and schedule async delivery if the
/// peer has started listening.
fn channel_post(from: u64, json: String) {
    let Some(peer) = CH_PEER.with(|m| m.borrow().get(&from).copied()) else {
        return;
    };
    CH_QUEUE.with(|q| q.borrow_mut().entry(peer).or_default().push_back(json));
    if CH_STARTED.with(|s| s.borrow().contains(&peer)) {
        schedule_channel_delivery(peer);
    }
}

/// Mark a port started and flush anything already queued to it.
fn start_channel_port(pid: u64) {
    let newly = CH_STARTED.with(|s| s.borrow_mut().insert(pid));
    if !newly {
        return;
    }
    let pending = CH_QUEUE.with(|q| q.borrow().get(&pid).map_or(0, |d| d.len()));
    for _ in 0..pending {
        schedule_channel_delivery(pid);
    }
}

/// Post one delivery `IoTask` onto THIS thread's loop; keep the loop alive until
/// it runs.
fn schedule_channel_delivery(pid: u64) {
    with_host(|h| h.incr_handle());
    let io = with_host(|h| h.io_sender());
    let _ = io.send(Box::new(move || channel_deliver(pid)));
}

/// Deliver (at most) one queued message to port `pid` by emitting `'message'`.
/// Always `Ok(())`: a listener error is caught, never `?`-propagated (that would
/// unwind the whole event loop — see the module docs).
fn channel_deliver(pid: u64) -> Result<(), String> {
    let json = CH_QUEUE.with(|q| q.borrow_mut().get_mut(&pid).and_then(|d| d.pop_front()));
    if let Some(json) = json {
        if let Some(port) = CHANNEL_PORTS.with(|m| m.borrow().get(&pid).cloned()) {
            match deserialize(&json) {
                Ok(v) => {
                    if let Err(e) = emit_event(&port, "message", vec![v]) {
                        eprintln!("{e}");
                    }
                }
                Err(e) => eprintln!("{e}"),
            }
        }
    }
    with_host(|h| h.decr_handle());
    Ok(())
}

/// `worker.receiveMessageOnPort(port)` → `{ message }` draining one queued message
/// from `port`, or `undefined` if none is queued.
fn receive_message_on_port(port: Option<&Value>) -> Value {
    let Some(port) = port else {
        return Value::Undef;
    };
    let Some(pid) = u64_prop(port, "@@portid") else {
        return Value::Undef;
    };
    let json = CH_QUEUE.with(|q| q.borrow_mut().get_mut(&pid).and_then(|d| d.pop_front()));
    match json {
        Some(j) => match deserialize(&j) {
            Ok(v) => with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("message".into(), v);
                h.new_object(m)
            }),
            Err(_) => Value::Undef,
        },
        None => Value::Undef,
    }
}

// ── BroadcastChannel (in-process, SAME-thread pub/sub by name) ────────────────

/// `new BroadcastChannel(name)` → an object that broadcasts `postMessage` data to
/// every OTHER `BroadcastChannel` of the same name.
///
/// LIMITATION (documented, never faked): this is SAME-THREAD only. Node's
/// BroadcastChannel spans worker threads; node-js delivers only to channels on
/// the constructing thread (cross-thread delivery would need the worker bridge and
/// is out of scope here). Like Node, a channel keeps the event loop alive (refs a
/// handle) until `close()` / `unref()`.
pub fn construct_broadcast_channel(args: &[Value]) -> Result<Value, String> {
    let name = with_host(|h| h.str_of(&arg0(args)));
    let id = NEXT_PORT_ID.fetch_add(1, Ordering::SeqCst);
    let obj = with_host(|h| {
        let listeners = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("BroadcastChannel"));
        m.insert("@@bcid".into(), Value::Float(id as f64));
        m.insert("@@bcname".into(), h.new_str(name.clone()));
        m.insert("@@listeners".into(), listeners);
        m.insert("@@refed".into(), Value::Bool(true));
        m.insert("name".into(), h.new_str(name.clone()));
        m.insert("onmessage".into(), h.null());
        m.insert("onmessageerror".into(), h.null());
        h.new_object(m)
    });
    BCAST.with(|b| {
        b.borrow_mut()
            .entry(name)
            .or_default()
            .push((id, obj.clone()))
    });
    with_host(|h| h.incr_handle());
    Ok(obj)
}

/// Method dispatch for a `BroadcastChannel` object.
fn broadcast_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        "postMessage" => {
            let json = serialize(&arg0(&args))?;
            let name = str_prop(recv, "@@bcname");
            let self_id = u64_prop(recv, "@@bcid");
            let targets: Vec<Value> = BCAST.with(|b| match b.borrow().get(&name) {
                Some(list) => list
                    .iter()
                    .filter(|(id, _)| Some(*id) != self_id)
                    .map(|(_, v)| v.clone())
                    .collect(),
                None => Vec::new(),
            });
            for t in targets {
                schedule_broadcast_delivery(t, json.clone());
            }
            Ok(Value::Undef)
        }
        "close" => {
            let name = str_prop(recv, "@@bcname");
            let self_id = u64_prop(recv, "@@bcid");
            BCAST.with(|b| {
                if let Some(list) = b.borrow_mut().get_mut(&name) {
                    list.retain(|(id, _)| Some(*id) != self_id);
                }
            });
            release_broadcast_ref(recv);
            Ok(Value::Undef)
        }
        "addEventListener" => {
            // Only 'message' is meaningful here; store the callback.
            let ev = with_host(|h| args.first().map(|v| h.str_of(v)).unwrap_or_default());
            if ev == "message" {
                if let Some(cb) = args.get(1) {
                    if let Some(arr) = get_prop(recv, "@@listeners") {
                        with_host(|h| {
                            if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
                                items.push(cb.clone());
                            }
                        });
                    }
                }
            }
            Ok(Value::Undef)
        }
        "removeEventListener" => Ok(Value::Undef),
        "ref" => {
            let refed = matches!(get_prop(recv, "@@refed"), Some(Value::Bool(true)));
            if !refed {
                with_host(|h| h.incr_handle());
                set_bool(recv, "@@refed", true);
            }
            Ok(recv.clone())
        }
        "unref" => {
            release_broadcast_ref(recv);
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!(
            "BroadcastChannel.{method} is not a function"
        ))),
    }
}

/// Drop the channel's event-loop handle if it currently holds one.
fn release_broadcast_ref(recv: &Value) {
    if matches!(get_prop(recv, "@@refed"), Some(Value::Bool(true))) {
        with_host(|h| h.decr_handle());
        set_bool(recv, "@@refed", false);
    }
}

/// Schedule delivery of a broadcast message to one target channel.
fn schedule_broadcast_delivery(target: Value, json: String) {
    with_host(|h| h.incr_handle());
    let io = with_host(|h| h.io_sender());
    let _ = io.send(Box::new(move || broadcast_deliver(target, json)));
}

/// Deliver a broadcast message: build a `MessageEvent`-like `{ data, type }` and
/// invoke the target's `onmessage` plus any `addEventListener('message')` handlers.
/// Always `Ok(())` — handler errors are caught, never `?`-propagated.
fn broadcast_deliver(target: Value, json: String) -> Result<(), String> {
    let value = match deserialize(&json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            with_host(|h| h.decr_handle());
            return Ok(());
        }
    };
    let event = with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("data".into(), value);
        m.insert("type".into(), h.new_str("message"));
        h.new_object(m)
    });
    let onmessage = get_prop(&target, "onmessage");
    let mut handlers: Vec<Value> = Vec::new();
    if let Some(cb) = onmessage {
        if with_host(|h| crate::host::is_callable(h, &cb)) {
            handlers.push(cb);
        }
    }
    if let Some(arr) = get_prop(&target, "@@listeners") {
        let listeners: Vec<Value> = with_host(|h| match h.get(&arr) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        });
        handlers.extend(listeners);
    }
    for cb in handlers {
        if let Err(e) = crate::host::invoke(&cb, vec![event.clone()], None) {
            eprintln!("{e}");
        }
    }
    with_host(|h| h.decr_handle());
    Ok(())
}

/// The string value of a hidden own property of `recv`.
fn str_prop(recv: &Value, key: &str) -> String {
    get_prop(recv, key)
        .map(|v| with_host(|h| h.str_of(&v)))
        .unwrap_or_default()
}

/// Set a boolean own property on `recv`.
fn set_bool(recv: &Value, key: &str, val: bool) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), Value::Bool(val));
        }
    });
}
