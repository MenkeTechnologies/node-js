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
];

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
        _ => return None,
    })
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
