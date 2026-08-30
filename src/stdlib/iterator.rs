//! Iterator helpers (27.1.4) — `map`, `filter`, `take`, `drop`, `flatMap` and
//! the terminal `reduce`/`toArray`/`forEach`/`some`/`every`/`find`, plus the
//! `Iterator` constructor and `Iterator.from`.
//!
//! None of it existed: `[1,2,3].values().map(f)` was "map is not a function".
//!
//! The helpers are LAZY, which is the whole point of them — `take(3)` on an
//! endless generator has to stop after three pulls, not materialise anything.
//! Each is an `@@native = "IteratorHelper"` object holding the iterator it
//! draws from, so a chain is a chain of pulls; only the terminal operations
//! drain. That also makes them work over any iterator, including a user object
//! with a `next` method.

use crate::host::{call_method, is_callable, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// The lazy helpers, which return another iterator.
pub const LAZY: &[&str] = &["map", "filter", "take", "drop", "flatMap"];

/// The terminal operations, which drain the iterator and return a value.
pub const TERMINAL: &[&str] = &["reduce", "toArray", "forEach", "some", "every", "find"];

/// Everything `Iterator.prototype` carries, so any iterator answers for it.
pub const METHODS: &[&str] = &[
    "map",
    "filter",
    "take",
    "drop",
    "flatMap",
    "reduce",
    "toArray",
    "forEach",
    "some",
    "every",
    "find",
    "next",
    "return",
    "@@iterator",
];

/// `Iterator`'s own statics.
pub const STATIC_METHODS: &[&str] = &["from"];

/// True if `name` is one of the helper methods (lazy or terminal).
pub fn is_helper(name: &str) -> bool {
    LAZY.contains(&name) || TERMINAL.contains(&name)
}

/// A lazy helper over `src`.
fn helper(src: &Value, op: &str, arg: Value) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("IteratorHelper"));
        m.insert("@@src".into(), src.clone());
        m.insert("@@op".into(), h.new_str(op));
        m.insert("@@arg".into(), arg);
        // `take`/`drop` count down; `flatMap` parks the inner iterator here.
        m.insert("@@count".into(), Value::Float(0.0));
        m.insert("@@done".into(), Value::Bool(false));
        h.new_object(m)
    })
}

fn slot(recv: &Value, k: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(k).cloned(),
        _ => None,
    })
}

fn set_slot(recv: &Value, k: &str, v: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(k.to_string(), v);
        }
    });
}

/// `{ value, done }`.
fn step(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("value".into(), value);
        m.insert("done".into(), Value::Bool(done));
        h.new_object(m)
    })
}

/// Mark a helper exhausted and close whatever it was drawing from — a `return`
/// on any stage of a chain has to reach the generator at the bottom of it.
pub fn helper_return(recv: &Value) -> Value {
    let src = slot(recv, "@@src").unwrap_or(Value::Undef);
    let already = slot(recv, "@@done").is_some_and(|v| with_host(|h| h.truthy(&v)));
    set_slot(recv, "@@done", Value::Bool(true));
    if !already {
        close(&src);
    }
    done_step()
}

/// The `{ value: undefined, done: true }` an exhausted iterator reports.
pub fn done_step() -> Value {
    step(Value::Undef, true)
}

/// Pull one step from an iterator of any kind, as `(value, done)`.
fn pull(it: &Value) -> Result<(Value, bool), String> {
    let r = call_method(it, "next", Vec::new())?;
    let done = crate::builtins::get_property(&r, "done")?;
    let done = with_host(|h| h.truthy(&done));
    let value = crate::builtins::get_property(&r, "value")?;
    Ok((value, done))
}

/// Close an iterator that is being abandoned early (7.4.9 IteratorClose): its
/// `return` runs, so a generator's `finally` block fires.
fn close(it: &Value) {
    // `get_property`, not `lookup_chain`: a generator's and a helper's `return`
    // resolve through the stdlib funnel rather than a property map, so the
    // chain read alone finds neither and every abandoned iterator stayed open.
    let f = crate::builtins::get_property(it, "return").unwrap_or(Value::Undef);
    if with_host(|h| is_callable(h, &f)) {
        let _ = call_method(it, "return", Vec::new());
    }
}

/// 27.1.4.x `ToIntegerOrInfinity` for `take`/`drop`'s limit, which must be a
/// non-negative number — `take(-1)` and `take(NaN)` are RangeErrors, not silent
/// no-ops.
fn limit_arg(args: &[Value]) -> Result<f64, String> {
    let raw = args.first().cloned().unwrap_or(Value::Undef);
    let n = with_host(|h| h.to_number(&raw));
    if n.is_nan() {
        return Err(crate::host::range_error("NaN must be positive"));
    }
    if n < 0.0 {
        let shown = with_host(|h| h.inspect(&Value::Float(n)));
        return Err(crate::host::range_error(&format!(
            "{shown} must be positive"
        )));
    }
    Ok(n.trunc())
}

/// A callable argument, or the `TypeError` a helper raises without one.
fn fn_arg(args: &[Value]) -> Result<Value, String> {
    let f = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| is_callable(h, &f)) {
        return Err(crate::host::type_error(
            &crate::host::not_a_function_message(&f),
        ));
    }
    Ok(f)
}

/// Dispatch a helper called on the iterator `recv`.
pub fn call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "map" | "filter" | "flatMap" => Ok(helper(recv, method, fn_arg(args)?)),
        "take" | "drop" => Ok(helper(recv, method, Value::Float(limit_arg(args)?))),
        "toArray" => {
            let mut out = Vec::new();
            loop {
                let (v, done) = pull(recv)?;
                if done {
                    break;
                }
                out.push(v);
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "forEach" => {
            let f = fn_arg(args)?;
            let mut i = 0.0;
            loop {
                let (v, done) = pull(recv)?;
                if done {
                    break;
                }
                crate::host::invoke(&f, vec![v, Value::Float(i)], None)?;
                i += 1.0;
            }
            Ok(Value::Undef)
        }
        "reduce" => {
            let f = fn_arg(args)?;
            let mut acc = args.get(1).cloned();
            let mut i = 0.0;
            loop {
                let (v, done) = pull(recv)?;
                if done {
                    break;
                }
                acc = Some(match acc {
                    // 27.1.4.11 step 5: with no seed the FIRST value becomes the
                    // accumulator and the reducer is not called for it.
                    None => v,
                    Some(a) => crate::host::invoke(&f, vec![a, v, Value::Float(i)], None)?,
                });
                i += 1.0;
            }
            acc.ok_or_else(|| {
                crate::host::type_error("Reduce of a done iterator with no initial value")
            })
        }
        "some" | "every" | "find" => {
            let f = fn_arg(args)?;
            let mut i = 0.0;
            loop {
                let (v, done) = pull(recv)?;
                if done {
                    break;
                }
                let r = crate::host::invoke(&f, vec![v.clone(), Value::Float(i)], None)?;
                let hit = with_host(|h| h.truthy(&r));
                // Each stops at the first decisive element and CLOSES the
                // iterator it abandoned.
                match method {
                    "some" if hit => {
                        close(recv);
                        return Ok(Value::Bool(true));
                    }
                    "every" if !hit => {
                        close(recv);
                        return Ok(Value::Bool(false));
                    }
                    "find" if hit => {
                        close(recv);
                        return Ok(v);
                    }
                    _ => {}
                }
                i += 1.0;
            }
            Ok(match method {
                "some" => Value::Bool(false),
                "every" => Value::Bool(true),
                _ => Value::Undef,
            })
        }
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// One step of a lazy helper: pull from the source until this stage produces a
/// value or the source runs out.
pub fn helper_next(recv: &Value) -> Result<Value, String> {
    if slot(recv, "@@done").is_some_and(|v| with_host(|h| h.truthy(&v))) {
        return Ok(step(Value::Undef, true));
    }
    let src = slot(recv, "@@src").unwrap_or(Value::Undef);
    let op = slot(recv, "@@op")
        .map(|v| with_host(|h| h.str_of(&v)))
        .unwrap_or_default();
    let arg = slot(recv, "@@arg").unwrap_or(Value::Undef);
    let finish = || {
        set_slot(recv, "@@done", Value::Bool(true));
        step(Value::Undef, true)
    };
    match op.as_str() {
        "take" => {
            let limit = with_host(|h| h.to_number(&arg));
            let seen = slot(recv, "@@count")
                .map(|v| with_host(|h| h.to_number(&v)))
                .unwrap_or(0.0);
            if seen >= limit {
                // The source is abandoned, so it is closed.
                close(&src);
                return Ok(finish());
            }
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            set_slot(recv, "@@count", Value::Float(seen + 1.0));
            Ok(step(v, false))
        }
        "drop" => {
            let limit = with_host(|h| h.to_number(&arg));
            let mut dropped = slot(recv, "@@count")
                .map(|v| with_host(|h| h.to_number(&v)))
                .unwrap_or(0.0);
            while dropped < limit {
                let (_, done) = pull(&src)?;
                dropped += 1.0;
                set_slot(recv, "@@count", Value::Float(dropped));
                if done {
                    return Ok(finish());
                }
            }
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            Ok(step(v, false))
        }
        "map" => {
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            let i = slot(recv, "@@count")
                .map(|x| with_host(|h| h.to_number(&x)))
                .unwrap_or(0.0);
            set_slot(recv, "@@count", Value::Float(i + 1.0));
            let out = crate::host::invoke(&arg, vec![v, Value::Float(i)], None)?;
            Ok(step(out, false))
        }
        "filter" => loop {
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            let i = slot(recv, "@@count")
                .map(|x| with_host(|h| h.to_number(&x)))
                .unwrap_or(0.0);
            set_slot(recv, "@@count", Value::Float(i + 1.0));
            let keep = crate::host::invoke(&arg, vec![v.clone(), Value::Float(i)], None)?;
            if with_host(|h| h.truthy(&keep)) {
                return Ok(step(v, false));
            }
        },
        "flatMap" => loop {
            // An inner iterator already in flight is drained first.
            if let Some(inner) = slot(recv, "@@inner") {
                if !matches!(inner, Value::Undef) {
                    let (v, done) = pull(&inner)?;
                    if !done {
                        return Ok(step(v, false));
                    }
                    set_slot(recv, "@@inner", Value::Undef);
                }
            }
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            let i = slot(recv, "@@count")
                .map(|x| with_host(|h| h.to_number(&x)))
                .unwrap_or(0.0);
            set_slot(recv, "@@count", Value::Float(i + 1.0));
            let mapped = crate::host::invoke(&arg, vec![v, Value::Float(i)], None)?;
            let inner = iterator_of(&mapped)?;
            set_slot(recv, "@@inner", inner);
        },
        // `Iterator.from`'s wrapper: forward each step unchanged.
        "wrap" => {
            let (v, done) = pull(&src)?;
            if done {
                return Ok(finish());
            }
            Ok(step(v, false))
        }
        _ => Ok(finish()),
    }
}

/// The iterator for a value, via its `Symbol.iterator` — what `flatMap` and
/// `Iterator.from` both need.
fn iterator_of(v: &Value) -> Result<Value, String> {
    // `get_property` rather than `lookup_chain`: an Array's or a String's
    // `Symbol.iterator` resolves through the stdlib funnel, not a property map,
    // so the chain read alone reports every builtin as non-iterable.
    let f = crate::builtins::get_property(v, "@@iterator").unwrap_or(Value::Undef);
    if with_host(|h| is_callable(h, &f)) {
        return call_method(v, "@@iterator", Vec::new());
    }
    // A raw iterator object — one with `next` but no `Symbol.iterator` — is
    // taken as-is, which is what `Iterator.from` accepts.
    let next = crate::builtins::get_property(v, "next").unwrap_or(Value::Undef);
    if with_host(|h| is_callable(h, &next)) {
        return Ok(v.clone());
    }
    Err(crate::host::type_error(&format!(
        "{} is not iterable",
        with_host(|h| h.inspect(v))
    )))
}

/// `Iterator.from(x)`.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match method {
        // `Iterator.from(x)` hands back something that HAS the helpers. A plain
        // object with a `next` method has none of its own, so 27.1.4.1 wraps it
        // — here in a pass-through helper, which is the same wrapper every
        // other stage uses.
        "from" => Some(
            iterator_of(&args.first().cloned().unwrap_or(Value::Undef)).map(|it| {
                if super::native_tag(&it).as_deref() == Some("IteratorHelper")
                    || matches!(
                        with_host(|h| h.kind_of(&it)),
                        Some(crate::host::ObjKind::Generator) | Some(crate::host::ObjKind::Iter)
                    )
                {
                    it
                } else {
                    helper(&it, "wrap", Value::Undef)
                }
            }),
        ),
        _ => None,
    }
}
