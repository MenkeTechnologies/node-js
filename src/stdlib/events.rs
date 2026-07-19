//! Node `events` module: `EventEmitter`. The emitter is an object tagged
//! `@@native = "EventEmitter"` with hidden `@@on`/`@@once` maps (event name →
//! listener array). `emit` collects listeners, releases the host borrow, then
//! invokes each so callbacks can re-enter the host.

use super::arg_str;
use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Construct a fresh `EventEmitter`.
pub fn new_emitter() -> Value {
    with_host(|h| {
        let on = h.new_object(IndexMap::new());
        let once = h.new_object(IndexMap::new());
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("EventEmitter"));
        m.insert("@@on".into(), on);
        m.insert("@@once".into(), once);
        h.new_object(m)
    })
}

pub fn instance_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        "on" | "addListener" | "prependListener" => {
            add(recv, "@@on", &arg_str(&args, 0), args.get(1).cloned().unwrap_or(Value::Undef));
            Ok(recv.clone())
        }
        "once" | "prependOnceListener" => {
            add(recv, "@@once", &arg_str(&args, 0), args.get(1).cloned().unwrap_or(Value::Undef));
            Ok(recv.clone())
        }
        "emit" => emit(recv, &arg_str(&args, 0), &args.get(1..).map(|s| s.to_vec()).unwrap_or_default()),
        "removeListener" | "off" => {
            remove(recv, &arg_str(&args, 0), args.get(1).cloned());
            Ok(recv.clone())
        }
        "removeAllListeners" => {
            let name = if args.is_empty() { None } else { Some(arg_str(&args, 0)) };
            remove_all(recv, name.as_deref());
            Ok(recv.clone())
        }
        "listenerCount" => Ok(Value::Float(listeners(recv, &arg_str(&args, 0)).len() as f64)),
        "eventNames" => Ok(with_host(|h| {
            let mut keys: Vec<String> = Vec::new();
            for map in ["@@on", "@@once"] {
                if let Some(JsObj::Object(p)) = named_map(h, recv, map).and_then(|v| h.get(&v)) {
                    keys.extend(p.keys().cloned());
                }
            }
            let names: Vec<Value> = keys.into_iter().map(|k| h.new_str(k)).collect();
            h.new_array(names)
        })),
        _ => Err(crate::host::type_error(&format!("emitter.{method} is not a function"))),
    }
}

fn named_map(h: &crate::host::JsHost, recv: &Value, which: &str) -> Option<Value> {
    match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(which).cloned(),
        _ => None,
    }
}

fn add(recv: &Value, which: &str, name: &str, f: Value) {
    with_host(|h| {
        let Some(map) = named_map(h, recv, which) else { return };
        // Ensure `map[name]` is an array, then push.
        let arr = match h.get(&map) {
            Some(JsObj::Object(p)) => p.get(name).cloned(),
            _ => None,
        };
        let arr = match arr {
            Some(a) if matches!(h.get(&a), Some(JsObj::Array(_))) => a,
            _ => {
                let a = h.new_array(Vec::new());
                if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                    p.insert(name.to_string(), a.clone());
                }
                a
            }
        };
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            items.push(f);
        }
    });
}

fn listeners(recv: &Value, name: &str) -> Vec<Value> {
    with_host(|h| {
        let mut out = Vec::new();
        for which in ["@@on", "@@once"] {
            if let Some(map) = named_map(h, recv, which) {
                if let Some(JsObj::Object(p)) = h.get(&map) {
                    if let Some(a) = p.get(name) {
                        if let Some(JsObj::Array(items)) = h.get(a) {
                            out.extend(items.iter().cloned());
                        }
                    }
                }
            }
        }
        out
    })
}

fn emit(recv: &Value, name: &str, args: &[Value]) -> Result<Value, String> {
    let to_call = listeners(recv, name);
    // Once-listeners fire a single time: clear them before invoking.
    remove_all_of(recv, "@@once", Some(name));
    let had = !to_call.is_empty();
    for f in to_call {
        invoke(&f, args.to_vec(), Some(recv.clone()))?;
    }
    Ok(Value::Bool(had))
}

fn remove(recv: &Value, name: &str, f: Option<Value>) {
    let Some(f) = f else { return };
    with_host(|h| {
        for which in ["@@on", "@@once"] {
            if let Some(map) = named_map(h, recv, which) {
                let arr = match h.get(&map) {
                    Some(JsObj::Object(p)) => p.get(name).cloned(),
                    _ => None,
                };
                if let Some(a) = arr {
                    if let Some(JsObj::Array(items)) = h.get_mut(&a) {
                        if let Some(pos) = items.iter().position(|x| x == &f) {
                            items.remove(pos);
                        }
                    }
                }
            }
        }
    });
}

fn remove_all(recv: &Value, name: Option<&str>) {
    remove_all_of(recv, "@@on", name);
    remove_all_of(recv, "@@once", name);
}

fn remove_all_of(recv: &Value, which: &str, name: Option<&str>) {
    with_host(|h| {
        if let Some(map) = named_map(h, recv, which) {
            if let Some(JsObj::Object(p)) = h.get_mut(&map) {
                match name {
                    Some(n) => {
                        p.shift_remove(n);
                    }
                    None => p.clear(),
                }
            }
        }
    });
}
