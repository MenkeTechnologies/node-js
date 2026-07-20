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
//!   sender thread:   value ─JSON.stringify([value])→ String  (on sender's heap)
//!   channel:         String  (Send)
//!   receiver thread: String ─JSON.parse(s)[0]→ value        (on receiver's heap)
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
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};

/// `worker_threads` exposes no module-level *functions* (its surface is the
/// `Worker` constructor plus the `isMainThread`/`threadId`/`parentPort`/
/// `workerData` constants). Kept for the `stdlib::is_method`/`call` contract.
pub const METHODS: &[&str] = &[];

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
}

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
        "Worker" => Some(with_host(|h| h.alloc(JsObj::Builtin("Worker".into())))),
        _ => None,
    }
}

/// Module-level dispatch. `worker_threads` has no callable module methods; kept
/// so `stdlib::call` can route uniformly.
pub fn call(_method: &str, _args: &[Value]) -> Option<Result<Value, String>> {
    None
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
        w.borrow_mut().insert(id, WorkerRec { emitter: emitter.clone(), to_worker: to_worker_tx });
    });
    // Keep the MAIN loop alive while the worker runs.
    with_host(|h| h.incr_handle());

    let spawn_tx = main_tx.clone();
    std::thread::spawn(move || {
        worker_thread_main(id, filename, is_eval, worker_data_json, spawn_tx, to_worker_rx);
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
    let Some(emitter) = emitter else { return Ok(()) };
    match ev {
        MainEvent::Online => emit_event(&emitter, "online", vec![]),
        MainEvent::Message(json) => {
            let v = deserialize(&json)?;
            emit_event(&emitter, "message", vec![v])
        }
        MainEvent::Error(msg) => {
            let err = crate::builtins::construct_builtin("Error", vec![with_host(|h| h.new_str(msg))])?;
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
    "on", "addListener", "prependListener", "once", "prependOnceListener", "emit",
    "removeListener", "off", "removeAllListeners", "listenerCount", "listeners",
    "eventNames", "setMaxListeners", "getMaxListeners",
];

pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        "Worker" => worker_call(recv, method, args),
        "MessagePort" => port_call(recv, method, args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
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
        _ => Err(crate::host::type_error(&format!("worker.{method} is not a function"))),
    }
}

/// Methods on the worker-side `parentPort` (`MessagePort`).
fn port_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
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
        _ => Err(crate::host::type_error(&format!("port.{method} is not a function"))),
    }
}
