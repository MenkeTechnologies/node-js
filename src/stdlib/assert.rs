//! Node `assert` module. Failing assertions throw an `AssertionError` (returned
//! as an `Err`, which the host surfaces as a thrown JS exception).

use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;

pub const METHODS: &[&str] = &[
    "ok",
    "equal",
    "notEqual",
    "strictEqual",
    "notStrictEqual",
    "deepEqual",
    "notDeepEqual",
    "deepStrictEqual",
    "notDeepStrictEqual",
    "throws",
    "doesNotThrow",
    "fail",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let a = || args.first().cloned().unwrap_or(Value::Undef);
    let b = || args.get(1).cloned().unwrap_or(Value::Undef);
    Some(match method {
        "ok" => assert_ok(args),
        "equal" => check(loose_eq(&a(), &b()), args, 2, "==", &a(), &b()),
        "notEqual" => check(!loose_eq(&a(), &b()), args, 2, "!=", &a(), &b()),
        "strictEqual" => check(strict(&a(), &b()), args, 2, "strictEqual", &a(), &b()),
        "notStrictEqual" => check(!strict(&a(), &b()), args, 2, "notStrictEqual", &a(), &b()),
        "deepEqual" => check(deep_equal(&a(), &b(), false), args, 2, "deepEqual", &a(), &b()),
        "notDeepEqual" => check(!deep_equal(&a(), &b(), false), args, 2, "notDeepEqual", &a(), &b()),
        "deepStrictEqual" => check(deep_equal(&a(), &b(), true), args, 2, "deepStrictEqual", &a(), &b()),
        "notDeepStrictEqual" => check(!deep_equal(&a(), &b(), true), args, 2, "notDeepStrictEqual", &a(), &b()),
        "throws" => throws(args, true),
        "doesNotThrow" => throws(args, false),
        "fail" => Err(fail_msg(args, 0, "Failed")),
        _ => return None,
    })
}

/// `assert(value[, message])` — throws unless `value` is truthy.
pub fn assert_ok(args: &[Value]) -> Result<Value, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    if with_host(|h| h.truthy(&v)) {
        Ok(Value::Undef)
    } else {
        Err(fail_msg(args, 1, "The expression evaluated to a falsy value"))
    }
}

fn check(pass: bool, args: &[Value], msg_idx: usize, op: &str, a: &Value, b: &Value) -> Result<Value, String> {
    if pass {
        return Ok(Value::Undef);
    }
    if let Some(m) = message(args, msg_idx) {
        return Err(assertion_error(&m));
    }
    let (sa, sb) = with_host(|h| (h.inspect(a), h.inspect(b)));
    Err(assertion_error(&format!("{sa} {op} {sb}")))
}

fn throws(args: &[Value], want_throw: bool) -> Result<Value, String> {
    let f = args.first().cloned().unwrap_or(Value::Undef);
    let threw = invoke(&f, Vec::new(), None).is_err();
    match (threw, want_throw) {
        (true, true) | (false, false) => Ok(Value::Undef),
        (false, true) => Err(assertion_error("Missing expected exception.")),
        (true, false) => Err(assertion_error("Got unwanted exception.")),
    }
}

fn message(args: &[Value], idx: usize) -> Option<String> {
    match args.get(idx) {
        Some(Value::Undef) | None => None,
        Some(v) => Some(with_host(|h| h.str_of(v))),
    }
}

fn fail_msg(args: &[Value], idx: usize, default: &str) -> String {
    assertion_error(&message(args, idx).unwrap_or_else(|| default.to_string()))
}

fn assertion_error(msg: &str) -> String {
    format!("AssertionError [ERR_ASSERTION]: {msg}")
}

fn strict(a: &Value, b: &Value) -> bool {
    with_host(|h| h.strict_eq(a, b))
}

fn loose_eq(a: &Value, b: &Value) -> bool {
    if strict(a, b) {
        return true;
    }
    with_host(|h| {
        let (na, nb) = (h.to_number(a), h.to_number(b));
        if !na.is_nan() && !nb.is_nan() && (na == nb) {
            return true;
        }
        h.str_of(a) == h.str_of(b)
    })
}

/// Structural equality. `strict` compares leaves with `===`, otherwise `==`.
pub fn deep_equal(a: &Value, b: &Value, strict_mode: bool) -> bool {
    let kinds = with_host(|h| {
        let av = h.get(a).map(kind);
        let bv = h.get(b).map(kind);
        (av, bv)
    });
    match kinds {
        (Some(Kind::Array), Some(Kind::Array)) => {
            let (ia, ib) = with_host(|h| (array_of(h, a), array_of(h, b)));
            ia.len() == ib.len() && ia.iter().zip(ib.iter()).all(|(x, y)| deep_equal(x, y, strict_mode))
        }
        (Some(Kind::Object), Some(Kind::Object)) => {
            let (ea, eb) = with_host(|h| (object_of(h, a), object_of(h, b)));
            if ea.len() != eb.len() {
                return false;
            }
            ea.iter().all(|(k, va)| {
                eb.iter().find(|(k2, _)| k2 == k).is_some_and(|(_, vb)| deep_equal(va, vb, strict_mode))
            })
        }
        _ => {
            if strict_mode {
                strict(a, b)
            } else {
                loose_eq(a, b)
            }
        }
    }
}

enum Kind {
    Array,
    Object,
}
fn kind(o: &JsObj) -> Kind {
    match o {
        JsObj::Array(_) => Kind::Array,
        _ => Kind::Object,
    }
}
fn array_of(h: &crate::host::JsHost, v: &Value) -> Vec<Value> {
    match h.get(v) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}
fn object_of(h: &crate::host::JsHost, v: &Value) -> Vec<(String, Value)> {
    match h.get(v) {
        Some(JsObj::Object(p)) => p.iter().filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#')).map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    }
}
