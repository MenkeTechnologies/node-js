//! Node `assert` module. Failing assertions throw an `AssertionError` (returned
//! as an `Err`, which the host surfaces as a thrown JS exception).

use crate::host::{
    call_method, invoke, is_callable, promise_of, reject_promise_val, resolve_promise_val,
    subscribe_native, take_exc_or_error, type_error, with_host, JsObj, PromiseState,
};
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
    "match",
    "doesNotMatch",
    "ifError",
    "partialDeepStrictEqual",
    "rejects",
    "doesNotReject",
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
        "match" => assert_match(args, true),
        "doesNotMatch" => assert_match(args, false),
        "ifError" => if_error(&a()),
        "partialDeepStrictEqual" => partial(&a(), &b(), args),
        "rejects" => Ok(rejects_impl(&a(), true)),
        "doesNotReject" => Ok(rejects_impl(&a(), false)),
        _ => return None,
    })
}

/// The strict-mode variants (`assert.strict.equal` === `assert.strictEqual`).
/// Maps the loose method names onto their strict counterparts, then delegates.
pub fn strict_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let mapped = match method {
        "equal" => "strictEqual",
        "notEqual" => "notStrictEqual",
        "deepEqual" => "deepStrictEqual",
        "notDeepEqual" => "notDeepStrictEqual",
        other => other,
    };
    call(mapped, args)
}

/// `assert.match(string, regexp)` / `assert.doesNotMatch(...)`. `regexp` must be
/// a `RegExp`; matching runs through the JS `RegExp.prototype.test`.
fn assert_match(args: &[Value], want_match: bool) -> Result<Value, String> {
    let s = args.first().cloned().unwrap_or(Value::Undef);
    let re = args.get(1).cloned().unwrap_or(Value::Undef);
    if !with_host(|h| matches!(h.get(&re), Some(JsObj::RegExp(_)))) {
        return Err(type_error(
            "The \"regexp\" argument must be an instance of RegExp.",
        ));
    }
    let matched = call_method(&re, "test", vec![s.clone()])?;
    let matched = with_host(|h| h.truthy(&matched));
    if matched == want_match {
        return Ok(Value::Undef);
    }
    if let Some(m) = message(args, 2) {
        return Err(assertion_error(&m));
    }
    let (sre, sstr) = with_host(|h| (h.inspect(&re), h.str_of(&s)));
    let verb = if want_match {
        "The input did not match the regular expression"
    } else {
        "The input was expected to not match the regular expression"
    };
    Err(assertion_error(&format!("{verb} {sre}. Input: '{sstr}'")))
}

/// `assert.ifError(value)` — throws unless `value` is `null`/`undefined`.
fn if_error(v: &Value) -> Result<Value, String> {
    if with_host(|h| h.is_nullish(v)) {
        return Ok(Value::Undef);
    }
    let desc = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get("message").map(|m| h.str_of(m)).unwrap_or_else(|| h.inspect(v)),
        _ => h.inspect(v),
    });
    Err(assertion_error(&format!("ifError got unwanted exception: {desc}")))
}

/// `assert.partialDeepStrictEqual(actual, expected)` — passes when every leaf of
/// `expected` strict-deep-matches the corresponding part of `actual` (extra
/// props/elements in `actual` are ignored).
fn partial(actual: &Value, expected: &Value, args: &[Value]) -> Result<Value, String> {
    if partial_deep(actual, expected) {
        return Ok(Value::Undef);
    }
    if let Some(m) = message(args, 2) {
        return Err(assertion_error(&m));
    }
    let (sa, sb) = with_host(|h| (h.inspect(actual), h.inspect(expected)));
    Err(assertion_error(&format!(
        "Expected values to be strictly deep-equal (partial):\n{sb} should be a subset of {sa}"
    )))
}

fn partial_deep(actual: &Value, expected: &Value) -> bool {
    let ekind = with_host(|h| h.get(expected).map(kind));
    match ekind {
        Some(Kind::Object) => {
            if !matches!(with_host(|h| h.get(actual).map(kind)), Some(Kind::Object)) {
                return false;
            }
            let (ea, ee) = with_host(|h| (object_of(h, actual), object_of(h, expected)));
            ee.iter().all(|(k, ve)| {
                ea.iter().find(|(k2, _)| k2 == k).is_some_and(|(_, va)| partial_deep(va, ve))
            })
        }
        Some(Kind::Array) => {
            if !matches!(with_host(|h| h.get(actual).map(kind)), Some(Kind::Array)) {
                return false;
            }
            let (ia, ie) = with_host(|h| (array_of(h, actual), array_of(h, expected)));
            ie.len() <= ia.len() && ie.iter().zip(ia.iter()).all(|(e, a)| partial_deep(a, e))
        }
        _ => strict(actual, expected),
    }
}

/// `assert.rejects(fn|promise)` / `assert.doesNotReject(...)` — returns a Promise
/// that fulfills when the operand settles the expected way, else rejects with an
/// `AssertionError`.
fn rejects_impl(input: &Value, want_reject: bool) -> Value {
    let result = with_host(|h| h.new_promise());
    let rid = with_host(|h| h.promise_id(&result).unwrap());
    // Reduce the operand to a promise: call it if it is a function.
    let operand = if with_host(|h| is_callable(h, input)) {
        match invoke(input, Vec::new(), None) {
            Ok(v) => promise_of(&v),
            Err(e) => {
                let ev = take_exc_or_error(&e);
                let p = with_host(|h| h.new_promise());
                let pid = with_host(|h| h.promise_id(&p).unwrap());
                reject_promise_val(pid, ev);
                p
            }
        }
    } else {
        promise_of(input)
    };
    let Some(oid) = with_host(|h| h.promise_id(&operand)) else {
        // Not thenable: treat as an immediate non-rejection.
        settle_rejects(rid, false, want_reject);
        return result;
    };
    subscribe_native(
        oid,
        Box::new(move |state, _val| {
            settle_rejects(rid, state == PromiseState::Rejected, want_reject);
            Ok(())
        }),
    );
    result
}

/// `new assert.AssertionError(options)` — a real `Error`-prototype-linked object
/// carrying `name`/`message`/`code`/`actual`/`expected`/`operator`. Parent wires
/// this to `construct("AssertionError")` and `constant("assert","AssertionError")`.
pub fn construct_assertion_error(args: &[Value]) -> Value {
    let opts = args.first().cloned().unwrap_or(Value::Undef);
    let (message, actual, expected, operator) = with_host(|h| match h.get(&opts) {
        Some(JsObj::Object(p)) => (
            p.get("message").map(|v| h.str_of(v)),
            p.get("actual").cloned(),
            p.get("expected").cloned(),
            p.get("operator").map(|v| h.str_of(v)),
        ),
        _ => (None, None, None, None),
    });
    let generated = message.is_none();
    let msg = message.unwrap_or_else(|| {
        let (sa, se) = with_host(|h| {
            (
                actual.as_ref().map(|v| h.inspect(v)).unwrap_or_default(),
                expected.as_ref().map(|v| h.inspect(v)).unwrap_or_default(),
            )
        });
        let op = operator.clone().unwrap_or_else(|| "==".to_string());
        format!("{sa} {op} {se}")
    });
    let stack = format!("AssertionError [ERR_ASSERTION]: {msg}\n    at <anonymous>");
    let op_val = operator.map(|o| with_host(|h| h.new_str(o))).unwrap_or(Value::Undef);
    let name_v = with_host(|h| h.new_str("AssertionError"));
    let msg_v = with_host(|h| h.new_str(msg));
    let code_v = with_host(|h| h.new_str("ERR_ASSERTION"));
    let stack_v = with_host(|h| h.new_str(stack));
    let mut props: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
    props.insert("name".into(), name_v);
    props.insert("message".into(), msg_v);
    props.insert("code".into(), code_v);
    props.insert("actual".into(), actual.unwrap_or(Value::Undef));
    props.insert("expected".into(), expected.unwrap_or(Value::Undef));
    props.insert("operator".into(), op_val);
    props.insert("generatedMessage".into(), Value::Bool(generated));
    props.insert("stack".into(), stack_v);
    let obj = with_host(|h| h.new_object(props));
    with_host(|h| {
        h.ensure_error_protos();
        if let Some(p) = crate::host::error_proto_of(h, "Error") {
            h.set_proto(&obj, p);
        }
    });
    obj
}

fn settle_rejects(rid: u32, rejected: bool, want_reject: bool) {
    if rejected == want_reject {
        resolve_promise_val(rid, Value::Undef);
    } else {
        let msg = if want_reject {
            "AssertionError [ERR_ASSERTION]: Missing expected rejection."
        } else {
            "AssertionError [ERR_ASSERTION]: Got unwanted rejection."
        };
        let ev = with_host(|h| crate::builtins::synth_error(h, msg));
        reject_promise_val(rid, ev);
    }
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
