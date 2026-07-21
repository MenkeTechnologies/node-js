//! Node `trace_events` module.
//!
//! Honest scope note: node-js has NO trace-event sink — it emits no
//! Chrome-trace / perfetto JSON and writes no `node_trace.*.log` file. What is
//! real here is the *object model and its state*: `createTracing({categories})`
//! returns a `Tracing` whose `.enable()`/`.disable()` genuinely flip its
//! `enabled` flag, whose `.categories` is the comma-joined category string, and
//! whose enabled categories are reflected by `getEnabledCategories()`. The
//! object behaves exactly as specified; it simply does not produce trace output.
//! This is a truthful "state, not sink" implementation, never a silent fake.

use crate::host::{type_error, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;

/// Module-level methods (`require('trace_events').createTracing(...)`).
pub const METHODS: &[&str] = &["createTracing", "getEnabledCategories"];

/// Instance methods on a `Tracing` object (`.enabled`/`.categories` are plain
/// data properties, not methods).
pub const TRACING_METHODS: &[&str] = &["enable", "disable"];

thread_local! {
    /// Category → number of currently-enabled `Tracing` objects that include it.
    /// A category is "enabled" while its count is > 0; insertion order drives the
    /// `getEnabledCategories()` listing.
    static ENABLED: RefCell<IndexMap<String, usize>> = RefCell::new(IndexMap::new());
}

// ── module dispatch ──────────────────────────────────────────────────────────

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        "createTracing" => Some(Ok(new_tracing(args.first()))),
        "getEnabledCategories" => Some(Ok(get_enabled_categories())),
        _ => None,
    }
}

/// `new trace_events.Tracing(...)` is not part of Node's public API, but the
/// constructor path builds the same object as `createTracing`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    Ok(new_tracing(args.first()))
}

/// Build a `Tracing` from an options object `{ categories: [...] }`. `categories`
/// (comma-joined) and `enabled` (starting `false`) are stored as real data props
/// so `tracing.categories` / `tracing.enabled` read directly.
fn new_tracing(options: Option<&Value>) -> Value {
    let cats = read_categories(options);
    with_host(|h| {
        let joined = h.new_str(cats.join(","));
        let cat_arr: Vec<Value> = cats.iter().map(|c| h.new_str(c.clone())).collect();
        let cat_arr = h.new_array(cat_arr);
        let mut m = IndexMap::new();
        m.insert("@@native".to_string(), h.new_str("Tracing"));
        m.insert("@@categories".to_string(), cat_arr);
        m.insert("categories".to_string(), joined);
        m.insert("enabled".to_string(), Value::Bool(false));
        h.new_object(m)
    })
}

/// `getEnabledCategories()` — the comma-joined set of categories enabled by any
/// live `Tracing`, or `undefined` when none are enabled.
fn get_enabled_categories() -> Value {
    let joined = ENABLED.with(|e| {
        e.borrow()
            .iter()
            .filter(|(_, &n)| n > 0)
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>()
            .join(",")
    });
    if joined.is_empty() {
        Value::Undef
    } else {
        with_host(|h| h.new_str(joined))
    }
}

// ── instance dispatch ────────────────────────────────────────────────────────

pub fn instance_call(recv: &Value, method: &str, _args: Vec<Value>) -> Result<Value, String> {
    match method {
        "enable" => {
            set_enabled(recv, true);
            Ok(recv.clone())
        }
        "disable" => {
            set_enabled(recv, false);
            Ok(recv.clone())
        }
        _ => Err(type_error(&format!("tracing.{method} is not a function"))),
    }
}

/// Flip the `enabled` flag and adjust the thread-local category counts. Toggling
/// to the state it is already in is a no-op (Node coalesces repeat calls).
fn set_enabled(recv: &Value, on: bool) {
    let already = matches!(get_prop(recv, "enabled"), Some(Value::Bool(true)));
    if already == on {
        return;
    }
    let cats = categories_of(recv);
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("enabled".to_string(), Value::Bool(on));
        }
    });
    ENABLED.with(|e| {
        let mut e = e.borrow_mut();
        for c in cats {
            let slot = e.entry(c).or_insert(0);
            if on {
                *slot += 1;
            } else if *slot > 0 {
                *slot -= 1;
            }
        }
    });
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

/// The `Tracing`'s categories (from its hidden `@@categories` array).
fn categories_of(recv: &Value) -> Vec<String> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => match p.get("@@categories").and_then(|a| h.get(a)) {
            Some(JsObj::Array(items)) => items.iter().map(|v| h.str_of(v)).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

/// Extract `options.categories` (an array of strings) from a `createTracing`
/// options object.
fn read_categories(options: Option<&Value>) -> Vec<String> {
    with_host(|h| {
        let Some(o) = options else { return Vec::new() };
        let Some(JsObj::Object(p)) = h.get(o) else {
            return Vec::new();
        };
        match p.get("categories").and_then(|c| h.get(c)) {
            Some(JsObj::Array(items)) => items.iter().map(|v| h.str_of(v)).collect(),
            _ => Vec::new(),
        }
    })
}
