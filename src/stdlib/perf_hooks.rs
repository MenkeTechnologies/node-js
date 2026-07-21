//! Node `perf_hooks` module.
//!
//! Exposes the `performance` object. `performance.now()` returns REAL monotonic
//! milliseconds elapsed since a process-start reference captured lazily in a
//! `OnceLock` (`std::time::Instant` — a true monotonic clock, never faked or
//! fuzzed). `timeOrigin` is the wall-clock time (Unix ms) at that same reference
//! point, so `timeOrigin + now()` approximates `Date.now()` as Node guarantees.
//!
//! `mark`/`measure`/`getEntriesByName`/`getEntriesByType`/`clearMarks` are
//! implemented against a small in-process entry buffer guarded by a `Mutex`. This
//! is best-effort: entries accumulate for the life of the process and the buffer
//! is not bounded (Node's PerformanceObserver / buffered-entry eviction is not
//! modeled), but marks and measures created and queried within a run behave
//! correctly.
//!
//! `performance` is surfaced as a `Builtin("performance")` namespace value, so
//! `performance.now()` dispatches through `call_method` → `call_builtin_function`
//! ("performance.now") → this module's `call`, and `performance.timeOrigin`
//! reads through `namespace_property` → this module's `constant`. The parent wires
//! BOTH the `perf_hooks` and `performance` namespaces to `call`/`constant` (see
//! the wiring note in the accompanying report).

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Methods available on both the `perf_hooks` module and its `performance`
/// object. (`timeOrigin` is a data property, served by `constant`.)
pub const METHODS: &[&str] = &[
    "now",
    "mark",
    "measure",
    "getEntriesByName",
    "getEntriesByType",
    "getEntries",
    "clearMarks",
    "clearMeasures",
    "createHistogram",
    "eventLoopUtilization",
    "monitorEventLoopDelay",
    "timerify",
    // Internal hook the `timerify` wrapper calls to deliver its 'function' entry
    // (hidden `@@` name — not a user-facing method, only reachable by the wrapper).
    "@@timerify_record",
];

/// Methods dispatched on an `@@native = "Histogram"` object (from
/// `createHistogram()` / `monitorEventLoopDelay()`; reported to the parent for
/// `instance_has_method` / `instance_call` wiring).
pub const HISTOGRAM_METHODS: &[&str] =
    &["record", "recordDelta", "reset", "percentile", "add", "enable", "disable"];

/// Methods dispatched on an `@@native = "PerformanceObserver"` object.
pub const PERFORMANCE_OBSERVER_METHODS: &[&str] = &["observe", "disconnect", "takeRecords"];

/// Methods dispatched on an `@@native = "PerformanceObserverEntryList"` object.
pub const OBSERVER_ENTRY_LIST_METHODS: &[&str] =
    &["getEntries", "getEntriesByName", "getEntriesByType"];

/// Node's sentinel `min` for an empty histogram (`i64::MAX`).
const EMPTY_HISTOGRAM_MIN: f64 = 9_223_372_036_854_775_807.0;

thread_local! {
    /// Live `PerformanceObserver` objects that should be notified when a mark or
    /// measure is recorded (only ever touched on the thread that owns them).
    static OBSERVERS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

/// The process-start reference: a monotonic `Instant` paired with the Unix-epoch
/// milliseconds at the same moment. Captured once, lazily.
struct Origin {
    instant: Instant,
    unix_ms: f64,
}

fn origin() -> &'static Origin {
    static ORIGIN: OnceLock<Origin> = OnceLock::new();
    ORIGIN.get_or_init(|| Origin {
        instant: Instant::now(),
        unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0),
    })
}

/// Real monotonic milliseconds since the process-start reference.
fn now_ms() -> f64 {
    origin().instant.elapsed().as_secs_f64() * 1000.0
}

/// A recorded performance entry (`PerformanceEntry` shape).
#[derive(Clone)]
struct Entry {
    name: String,
    entry_type: &'static str,
    start_time: f64,
    duration: f64,
}

/// The in-process entry buffer (marks + measures), in insertion order.
fn entries() -> &'static Mutex<Vec<Entry>> {
    static ENTRIES: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Non-function properties of `perf_hooks` / `performance`.
///
/// `perf_hooks.performance` → the `performance` namespace. `performance.timeOrigin`
/// → the fixed Unix-ms origin. `perf_hooks.constants` → a (currently empty) map.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "performance" => Some(with_host(|h| h.alloc(JsObj::Builtin("performance".into())))),
        "timeOrigin" => Some(Value::Float(origin().unix_ms)),
        "constants" => Some(with_host(|h| h.new_object(IndexMap::new()))),
        // Constructor names, exposed as values so `require('perf_hooks').X`
        // resolves and `typeof X === 'function'` holds. Only `PerformanceObserver`
        // is meaningfully instantiable here (see `construct`); the others exist for
        // name/`instanceof` resolution. Parent wires `PerformanceObserver`
        // construction into `construct`.
        "Performance"
        | "PerformanceEntry"
        | "PerformanceMark"
        | "PerformanceMeasure"
        | "PerformanceObserver"
        | "PerformanceObserverEntryList"
        | "PerformanceResourceTiming" => Some(with_host(|h| h.alloc(JsObj::Builtin(name.into())))),
        _ => None,
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "now" => Ok(Value::Float(now_ms())),
        "mark" => Ok(mark(args)),
        "measure" => Ok(measure(args)),
        "getEntries" => Ok(entries_to_array(|_| true)),
        "getEntriesByName" => {
            let name = super::arg_str(args, 0);
            // Optional second arg filters by entryType (an explicit `undefined`
            // means "no filter", matching Node).
            let ty = match args.get(1) {
                Some(v) if !matches!(v, Value::Undef) => Some(super::arg_str(args, 1)),
                _ => None,
            };
            Ok(entries_to_array(|e| {
                e.name == name && ty.as_deref().map(|t| t == e.entry_type).unwrap_or(true)
            }))
        }
        "getEntriesByType" => {
            let ty = super::arg_str(args, 0);
            Ok(entries_to_array(|e| e.entry_type == ty))
        }
        "clearMarks" => Ok(clear("mark", args)),
        "clearMeasures" => Ok(clear("measure", args)),
        // A real histogram over recorded values (see `histogram_instance_call`).
        "createHistogram" => Ok(new_histogram()),
        "eventLoopUtilization" => Ok(event_loop_utilization(args)),
        // A histogram-shaped monitor. LIMITATION: node-js has no background event-
        // loop-delay sampler, so this histogram accumulates no samples on its own
        // (it stays empty until values are `record`ed manually). `enable`/`disable`
        // are no-ops. Honest empty data, never a fabricated delay distribution.
        "monitorEventLoopDelay" => Ok(new_histogram()),
        "timerify" => timerify(args),
        "@@timerify_record" => Ok(timerify_record(args)),
        _ => return None,
    })
}

// ── timerify ──────────────────────────────────────────────────────────────────
// `performance.timerify(fn)` wraps `fn` so each call records a 'function'
// PerformanceEntry (name = `fn.name`, duration = call time). The wrapper is a REAL
// JS closure compiled + invoked here (the same re-entrant factory technique
// `util.promisify` uses), closing over the original function and a native record
// hook (`Builtin("performance.@@timerify_record")`). Node delivers 'function'
// entries to subscribed PerformanceObservers only — they are NOT retained on the
// global timeline (`performance.getEntriesByType('function')` is empty in v26) — so
// the hook notifies observers without buffering the entry.

/// Compile a single JS expression and run it on the current host, returning its
/// completion value (re-entrant-safe; mirrors `util`'s `run_completion`).
fn run_completion(src: &str) -> Result<Value, String> {
    let prog = crate::compile_completion(src)?;
    let chunk = crate::load_merged(prog);
    crate::host::run_chunk_on(chunk)
}

const TIMERIFY_SRC: &str = "(function(original, record){\n\
  var perf = require('perf_hooks').performance;\n\
  return function(){\n\
    var start = perf.now();\n\
    try {\n\
      return original.apply(this, arguments);\n\
    } finally {\n\
      record(original.name || '', start, perf.now());\n\
    }\n\
  };\n\
})";

/// `performance.timerify(fn[, options])` → a wrapped `fn` that records a 'function'
/// `PerformanceEntry` (its call duration) on every invocation.
fn timerify(args: &[Value]) -> Result<Value, String> {
    let orig = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &orig)) {
        return Err(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"fn\" argument must be of type function".into(),
        );
    }
    let factory = run_completion(TIMERIFY_SRC)?;
    let record = with_host(|h| h.alloc(JsObj::Builtin("performance.@@timerify_record".into())));
    crate::host::invoke(&factory, vec![orig, record], None)
}

/// Native hook invoked by the `timerify` wrapper `(name, startTime, endTime)`:
/// deliver a 'function' entry (call duration) to subscribed observers. Not stored
/// on the global timeline — matching Node v26, where function entries reach
/// observers only.
fn timerify_record(args: &[Value]) -> Value {
    let name = super::arg_str(args, 0);
    let start = super::arg_num(args, 1);
    let end = super::arg_num(args, 2);
    let e = Entry {
        name,
        entry_type: "function",
        start_time: start,
        duration: (end - start).max(0.0),
    };
    notify_observers(&e);
    Value::Undef
}

/// `new PerformanceObserver(callback)` — build an observer holding its callback,
/// its subscribed entry types, and a pending-entries buffer. Reported to the
/// parent for `construct` wiring.
pub fn construct(name: &str, args: &[Value]) -> Result<Value, String> {
    match name {
        "PerformanceObserver" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            Ok(with_host(|h| {
                let types = h.new_array(Vec::new());
                let buffer = h.new_array(Vec::new());
                let mut m = IndexMap::new();
                m.insert("@@native".into(), h.new_str("PerformanceObserver"));
                m.insert("@@cb".into(), cb);
                m.insert("@@types".into(), types);
                m.insert("@@buffer".into(), buffer);
                h.new_object(m)
            }))
        }
        _ => Err(crate::host::type_error(&format!("perf_hooks.{name} is not a constructor"))),
    }
}

/// `performance.mark(name)`: record a mark entry at the current time and return a
/// `PerformanceEntry` for it.
fn mark(args: &[Value]) -> Value {
    let name = super::arg_str(args, 0);
    let start = now_ms();
    let e = Entry { name, entry_type: "mark", start_time: start, duration: 0.0 };
    if let Ok(mut buf) = entries().lock() {
        buf.push(e.clone());
    }
    notify_observers(&e);
    entry_object(&e)
}

/// `performance.measure(name, startMark, endMark)`: record a measure spanning two
/// previously recorded marks (missing marks default to `0`/now), returning its
/// `PerformanceEntry`.
fn measure(args: &[Value]) -> Value {
    let name = super::arg_str(args, 0);
    let start_mark = args.get(1).map(|_| super::arg_str(args, 1));
    let end_mark = args.get(2).map(|_| super::arg_str(args, 2));
    let mark_time = |m: &Option<String>, default: f64| -> f64 {
        match m {
            Some(n) => entries()
                .lock()
                .ok()
                .and_then(|b| b.iter().rev().find(|e| e.entry_type == "mark" && &e.name == n).map(|e| e.start_time))
                .unwrap_or(default),
            None => default,
        }
    };
    let start = mark_time(&start_mark, 0.0);
    let end = mark_time(&end_mark, now_ms());
    let e = Entry {
        name,
        entry_type: "measure",
        start_time: start,
        duration: (end - start).max(0.0),
    };
    if let Ok(mut buf) = entries().lock() {
        buf.push(e.clone());
    }
    notify_observers(&e);
    entry_object(&e)
}

/// `clearMarks([name])` / `clearMeasures([name])`: drop entries of the given kind
/// (all, or only those named `name` when a name is supplied). Returns undefined.
fn clear(kind: &'static str, args: &[Value]) -> Value {
    let name = args.first().map(|_| super::arg_str(args, 0));
    if let Ok(mut buf) = entries().lock() {
        buf.retain(|e| {
            if e.entry_type != kind {
                return true;
            }
            match &name {
                Some(n) => &e.name != n,
                None => false,
            }
        });
    }
    Value::Undef
}

/// Build a JS array of `PerformanceEntry` objects for the buffered entries
/// matching `pred`.
fn entries_to_array(pred: impl Fn(&Entry) -> bool) -> Value {
    let matched: Vec<Entry> = entries()
        .lock()
        .map(|b| b.iter().filter(|e| pred(e)).cloned().collect())
        .unwrap_or_default();
    with_host(|h| {
        let items: Vec<Value> = matched.iter().map(|e| entry_object_h(h, e)).collect();
        h.new_array(items)
    })
}

/// Allocate a `PerformanceEntry`-shaped object.
fn entry_object(e: &Entry) -> Value {
    with_host(|h| entry_object_h(h, e))
}

fn entry_object_h(h: &mut crate::host::JsHost, e: &Entry) -> Value {
    let mut m = IndexMap::new();
    m.insert("name".into(), h.new_str(e.name.clone()));
    m.insert("entryType".into(), h.new_str(e.entry_type));
    m.insert("startTime".into(), Value::Float(e.start_time));
    m.insert("duration".into(), Value::Float(e.duration));
    h.new_object(m)
}

// ── histogram (createHistogram / monitorEventLoopDelay) ───────────────────────

/// A fresh, empty histogram object. Recorded values accumulate in the hidden
/// `@@vals` array; the `count`/`min`/`max`/`mean`/`stddev`/`exceeds` data
/// properties are kept in sync on every `record`, so a plain property read
/// (`h.min`) returns the right value without a getter.
fn new_histogram() -> Value {
    with_host(|h| {
        let vals = h.new_array(Vec::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Histogram"));
        m.insert("@@vals".into(), vals);
        m.insert("count".into(), Value::Float(0.0));
        m.insert("min".into(), Value::Float(EMPTY_HISTOGRAM_MIN));
        m.insert("max".into(), Value::Float(0.0));
        m.insert("mean".into(), Value::Float(f64::NAN));
        m.insert("stddev".into(), Value::Float(f64::NAN));
        m.insert("exceeds".into(), Value::Float(0.0));
        h.new_object(m)
    })
}

/// Dispatch a method on a `Histogram` instance (`@@native = "Histogram"`).
pub fn histogram_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "record" => {
            let n = super::arg_num(args, 0);
            push_value(recv, n);
            update_stats(recv);
            Ok(Value::Undef)
        }
        // Record the elapsed time (ms) since the previous `recordDelta` (or since
        // the histogram was created, for the first call).
        "recordDelta" => {
            let now = now_ms();
            let last = read_hidden_num(recv, "@@last").unwrap_or(now);
            set_hidden_num(recv, "@@last", now);
            if read_hidden_num(recv, "@@last_seen").is_some() {
                push_value(recv, now - last);
                update_stats(recv);
            }
            set_hidden_num(recv, "@@last_seen", 1.0);
            Ok(Value::Undef)
        }
        "reset" => {
            with_host(|h| {
                if let Some(vals) = hidden(recv, "@@vals") {
                    if let Some(JsObj::Array(items)) = h.get_mut(&vals) {
                        items.clear();
                    }
                }
            });
            update_stats(recv);
            Ok(Value::Undef)
        }
        "percentile" => {
            let p = super::arg_num(args, 0);
            Ok(Value::Float(percentile(recv, p)))
        }
        "add" => {
            // Merge another histogram's recorded values into this one.
            if let Some(other) = args.first() {
                for v in histogram_values(other) {
                    push_value(recv, v);
                }
                update_stats(recv);
            }
            Ok(Value::Undef)
        }
        // Interval-form controls: no background sampler to toggle (see the
        // `monitorEventLoopDelay` note). Accepted for compatibility.
        "enable" | "disable" => Ok(Value::Bool(true)),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// Push a recorded value onto the histogram's `@@vals` array.
fn push_value(recv: &Value, n: f64) {
    with_host(|h| {
        let v = Value::Float(n);
        if let Some(vals) = match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@vals").cloned(),
            _ => None,
        } {
            if let Some(JsObj::Array(items)) = h.get_mut(&vals) {
                items.push(v);
            }
        }
    });
}

/// The recorded values of any histogram object, as `f64`s.
fn histogram_values(recv: &Value) -> Vec<f64> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@vals").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.iter().map(|v| h.to_number(v)).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

/// Recompute `count`/`min`/`max`/`mean`/`stddev` from `@@vals` and write them back.
fn update_stats(recv: &Value) {
    let vals = histogram_values(recv);
    let (count, min, max, mean, stddev) = if vals.is_empty() {
        (0.0, EMPTY_HISTOGRAM_MIN, 0.0, f64::NAN, f64::NAN)
    } else {
        let n = vals.len() as f64;
        let sum: f64 = vals.iter().sum();
        let mean = sum / n;
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (n, min, max, mean, var.sqrt())
    };
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("count".into(), Value::Float(count));
            p.insert("min".into(), Value::Float(min));
            p.insert("max".into(), Value::Float(max));
            p.insert("mean".into(), Value::Float(mean));
            p.insert("stddev".into(), Value::Float(stddev));
        }
    });
}

/// Nearest-rank percentile of the recorded values (empty → 0), matching Node's
/// integer-valued percentile results for small samples.
fn percentile(recv: &Value, p: f64) -> f64 {
    let mut vals = histogram_values(recv);
    if vals.is_empty() {
        return 0.0;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = vals.len();
    let rank = (p / 100.0 * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    vals[idx]
}

/// A hidden own property of `recv`, if present.
fn hidden(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn read_hidden_num(recv: &Value, key: &str) -> Option<f64> {
    hidden(recv, key).map(|v| with_host(|h| h.to_number(&v)))
}

fn set_hidden_num(recv: &Value, key: &str, n: f64) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), Value::Float(n));
        }
    });
}

// ── eventLoopUtilization ──────────────────────────────────────────────────────

/// `performance.eventLoopUtilization([util1[, util2]])` → `{ idle, active,
/// utilization }`.
///
/// LIMITATION (documented, not faked): node-js does not separately instrument the
/// event loop's idle vs active time. The honest best-effort is `active` = total
/// milliseconds elapsed since process start (real uptime) and `idle` = 0, so
/// `utilization` = 1. When a prior result is passed, the numbers are the delta
/// between it and now (Node's diff form).
fn event_loop_utilization(args: &[Value]) -> Value {
    let active_now = now_ms();
    let (prev_idle, prev_active) = match args.first() {
        Some(prev) => (
            hidden_num(prev, "idle").unwrap_or(0.0),
            hidden_num(prev, "active").unwrap_or(0.0),
        ),
        None => (0.0, 0.0),
    };
    let idle = 0.0 - prev_idle;
    let active = active_now - prev_active;
    let denom = idle + active;
    let utilization = if denom > 0.0 { active / denom } else { 0.0 };
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("idle".into(), Value::Float(idle));
        m.insert("active".into(), Value::Float(active));
        m.insert("utilization".into(), Value::Float(utilization));
        h.new_object(m)
    })
}

fn hidden_num(recv: &Value, key: &str) -> Option<f64> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.to_number(v)),
        _ => None,
    })
}

// ── PerformanceObserver ───────────────────────────────────────────────────────

/// Dispatch a method on a `PerformanceObserver` instance.
pub fn observer_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        // `observe({ entryTypes: [...] } | { type: '...' })`: record the subscribed
        // types and register so future marks/measures notify this observer.
        "observe" => {
            let opts = args.first().cloned().unwrap_or(Value::Undef);
            let types = observe_types(&opts);
            with_host(|h| {
                let items: Vec<Value> = types.iter().map(|t| h.new_str(t.clone())).collect();
                let arr = h.new_array(items);
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("@@types".into(), arr);
                }
            });
            OBSERVERS.with(|o| {
                let mut list = o.borrow_mut();
                if !list.iter().any(|v| same_ref(v, recv)) {
                    list.push(recv.clone());
                }
            });
            Ok(Value::Undef)
        }
        "disconnect" => {
            OBSERVERS.with(|o| o.borrow_mut().retain(|v| !same_ref(v, recv)));
            with_host(|h| {
                if let Some(buf) = match h.get(recv) {
                    Some(JsObj::Object(p)) => p.get("@@buffer").cloned(),
                    _ => None,
                } {
                    if let Some(JsObj::Array(items)) = h.get_mut(&buf) {
                        items.clear();
                    }
                }
            });
            Ok(Value::Undef)
        }
        // Drain and return the observer's buffered entries.
        "takeRecords" => {
            let taken: Vec<Value> = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => match p.get("@@buffer").and_then(|a| h.get(a)) {
                    Some(JsObj::Array(items)) => items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            });
            with_host(|h| {
                if let Some(buf) = match h.get(recv) {
                    Some(JsObj::Object(p)) => p.get("@@buffer").cloned(),
                    _ => None,
                } {
                    if let Some(JsObj::Array(items)) = h.get_mut(&buf) {
                        items.clear();
                    }
                }
            });
            Ok(with_host(|h| h.new_array(taken)))
        }
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// The entry types an `observe(options)` call subscribes to (`entryTypes` array
/// or a single `type`).
fn observe_types(opts: &Value) -> Vec<String> {
    with_host(|h| match h.get(opts) {
        Some(JsObj::Object(p)) => {
            if let Some(JsObj::Array(items)) = p.get("entryTypes").and_then(|a| h.get(a)) {
                items.iter().map(|v| h.str_of(v)).collect()
            } else if let Some(t) = p.get("type") {
                vec![h.str_of(t)]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    })
}

/// Deliver a just-recorded entry to every subscribed observer.
///
/// DEVIATION (documented): Node batches entries and delivers them to the observer
/// callback asynchronously on a microtask. node-js delivers SYNCHRONOUSLY, one
/// entry per notification, right after the mark/measure is recorded. The callback
/// receives `(entryList, observer)` exactly as Node's does.
fn notify_observers(e: &Entry) {
    let observers: Vec<Value> = OBSERVERS.with(|o| o.borrow().clone());
    if observers.is_empty() {
        return;
    }
    for obs in observers {
        let types: Vec<String> = with_host(|h| match h.get(&obs) {
            Some(JsObj::Object(p)) => match p.get("@@types").and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => items.iter().map(|v| h.str_of(v)).collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        });
        if !types.iter().any(|t| t == e.entry_type) {
            continue;
        }
        // Buffer the entry on the observer, then invoke its callback with a
        // single-entry list.
        let entry = entry_object(e);
        with_host(|h| {
            if let Some(buf) = match h.get(&obs) {
                Some(JsObj::Object(p)) => p.get("@@buffer").cloned(),
                _ => None,
            } {
                if let Some(JsObj::Array(items)) = h.get_mut(&buf) {
                    items.push(entry);
                }
            }
        });
        let cb = with_host(|h| match h.get(&obs) {
            Some(JsObj::Object(p)) => p.get("@@cb").cloned(),
            _ => None,
        });
        let Some(cb) = cb else { continue };
        let list = entry_list_object(vec![entry_object(e)]);
        let _ = crate::host::invoke(&cb, vec![list, obs.clone()], None);
    }
}

/// Build a `PerformanceObserverEntryList` wrapping `items`.
fn entry_list_object(items: Vec<Value>) -> Value {
    with_host(|h| {
        let arr = h.new_array(items);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("PerformanceObserverEntryList"));
        m.insert("@@entries".into(), arr);
        h.new_object(m)
    })
}

/// Dispatch a method on a `PerformanceObserverEntryList` instance.
pub fn entry_list_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    let items: Vec<Value> = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@entries").and_then(|a| h.get(a)) {
            Some(JsObj::Array(v)) => v.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    });
    let prop = |v: &Value, key: &str| with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get(key).map(|x| h.str_of(x)),
        _ => None,
    });
    match method {
        "getEntries" => Ok(with_host(|h| h.new_array(items))),
        "getEntriesByName" => {
            let name = super::arg_str(args, 0);
            let ty = match args.get(1) {
                Some(v) if !matches!(v, Value::Undef) => Some(super::arg_str(args, 1)),
                _ => None,
            };
            let filtered: Vec<Value> = items
                .into_iter()
                .filter(|it| {
                    prop(it, "name").as_deref() == Some(name.as_str())
                        && ty.as_deref()
                            .map(|t| prop(it, "entryType").as_deref() == Some(t))
                            .unwrap_or(true)
                })
                .collect();
            Ok(with_host(|h| h.new_array(filtered)))
        }
        "getEntriesByType" => {
            let ty = super::arg_str(args, 0);
            let filtered: Vec<Value> = items
                .into_iter()
                .filter(|it| prop(it, "entryType").as_deref() == Some(ty.as_str()))
                .collect();
            Ok(with_host(|h| h.new_array(filtered)))
        }
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

/// Heap-identity comparison for two reference values.
fn same_ref(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Obj(x), Value::Obj(y)) if x == y)
}
