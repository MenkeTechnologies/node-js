//! Node `assert` module. Failing assertions throw an `AssertionError` (returned
//! as an `Err`, which the host surfaces as a thrown JS exception).

use crate::host::{
    call_method, invoke, is_callable, reject_promise_val, resolve_promise_val, subscribe_native,
    take_exc_or_error, with_host, JsObj, PromiseState,
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
        "strictEqual" => check(strict(&a(), &b()), args, 2, "===", &a(), &b()),
        "notStrictEqual" => check(!strict(&a(), &b()), args, 2, "!==", &a(), &b()),
        "deepEqual" => check(
            deep_equal(&a(), &b(), false),
            args,
            2,
            "deepEqual",
            &a(),
            &b(),
        ),
        "notDeepEqual" => check(
            !deep_equal(&a(), &b(), false),
            args,
            2,
            "notDeepEqual",
            &a(),
            &b(),
        ),
        "deepStrictEqual" => check(
            deep_equal(&a(), &b(), true),
            args,
            2,
            "deepStrictEqual",
            &a(),
            &b(),
        ),
        "notDeepStrictEqual" => check(
            !deep_equal(&a(), &b(), true),
            args,
            2,
            "notDeepStrictEqual",
            &a(),
            &b(),
        ),
        "throws" => throws(args, true),
        "doesNotThrow" => throws(args, false),
        // `fail` carries no operands: `actual`/`expected` are own properties
        // holding `undefined`, and `operator` is the literal `"fail"`.
        "fail" => Err(throw_assertion(
            &message(args, 0).unwrap_or_else(|| "Failed".to_string()),
            message(args, 0).is_none(),
            "fail",
            Value::Undef,
            Value::Undef,
        )),
        "match" => assert_match(args, true),
        "doesNotMatch" => assert_match(args, false),
        "ifError" => if_error(&a()),
        "partialDeepStrictEqual" => partial(&a(), &b(), args),
        "rejects" => Ok(rejects_impl(args, true)),
        "doesNotReject" => Ok(rejects_impl(args, false)),
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
    if !is_regexp(&re) {
        // Node names the instance, not a type, and appends the received value.
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"regexp\" argument must be an instance of RegExp. Received {}",
                crate::stdlib::received_desc(&re)
            ),
        ));
    }
    // `internalMatch`: a non-string input fails either assertion, and the
    // test is `RegExp.prototype.exec`, which honours `lastIndex`.
    let is_string = with_host(|h| h.type_of(&s) == "string");
    if is_string && regexp_matches(&re, &s)? == want_match {
        return Ok(Value::Undef);
    }
    let operator = if want_match { "match" } else { "doesNotMatch" };
    let custom = message(args, 2);
    let generated = custom.is_none();
    let msg = custom.unwrap_or_else(|| {
        with_host(|h| {
            if !is_string {
                return format!(
                    "The \"string\" argument must be of type string. Received type {} ({})",
                    h.type_of(&s),
                    h.inspect(&s)
                );
            }
            let verb = if want_match {
                "The input did not match the regular expression"
            } else {
                "The input was expected to not match the regular expression"
            };
            format!("{verb} {}. Input:\n\n{}\n", h.inspect(&re), h.inspect(&s))
        })
    });
    Err(throw_assertion(&msg, generated, operator, s, re))
}

fn is_regexp(v: &Value) -> bool {
    with_host(|h| matches!(h.get(v), Some(JsObj::RegExp(_))))
}

/// `RegExpPrototypeExec(re, s) !== null`.
fn regexp_matches(re: &Value, s: &Value) -> Result<bool, String> {
    let m = call_method(re, "exec", vec![s.clone()])?;
    Ok(!with_host(|h| h.is_null(&m)) && !matches!(m, Value::Undef))
}

/// `assert.ifError(value)` — throws unless `value` is `null`/`undefined`.
fn if_error(v: &Value) -> Result<Value, String> {
    if with_host(|h| h.is_nullish(v)) {
        return Ok(Value::Undef);
    }
    let desc = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p
            .get("message")
            .map(|m| h.str_of(m))
            .unwrap_or_else(|| h.inspect(v)),
        _ => h.inspect(v),
    });
    Err(assertion_error(&format!(
        "ifError got unwanted exception: {desc}"
    )))
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
                ea.iter()
                    .find(|(k2, _)| k2 == k)
                    .is_some_and(|(_, va)| partial_deep(va, ve))
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

/// `assert.rejects(fn|promise[, error][, message])` / `assert.doesNotReject(...)`
/// — node's `waitForActual` then `expectsError`/`expectsNoError`, so the
/// `error` argument validates a rejection exactly as it validates a throw.
/// Returns the promise those async functions return.
fn rejects_impl(args: &[Value], want_reject: bool) -> Value {
    let input = args.first().cloned().unwrap_or(Value::Undef);
    let rest: Vec<Value> = args.get(1..).unwrap_or(&[]).to_vec();
    let result = with_host(|h| h.new_promise());
    let rid = with_host(|h| h.promise_id(&result).unwrap());
    let fail = |e: String| {
        let ev = take_exc_or_error(&e);
        reject_promise_val(rid, ev);
    };
    // A function is called and must hand back a promise; anything else must
    // be one. A synchronous throw rejects this promise with that error, as an
    // `async function` would.
    let operand = if with_host(|h| is_callable(h, &input)) {
        match invoke(&input, Vec::new(), None) {
            Ok(v) if is_promise_like(&v) => crate::host::promise_of(&v),
            Ok(v) => {
                fail(crate::host::coded_error(
                    "TypeError",
                    "ERR_INVALID_RETURN_VALUE",
                    &format!(
                        "Expected instance of Promise to be returned from the \"promiseFn\" function but got {}.",
                        crate::stdlib::received_desc(&v)
                    ),
                ));
                return result;
            }
            Err(e) => {
                fail(e);
                return result;
            }
        }
    } else if is_promise_like(&input) {
        crate::host::promise_of(&input)
    } else {
        fail(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"promiseFn\" argument must be of type function or an instance of Promise. Received {}",
                crate::stdlib::received_desc(&input)
            ),
        ));
        return result;
    };
    let oid = with_host(|h| h.promise_id(&operand).unwrap());
    subscribe_native(
        oid,
        Box::new(move |state, val| {
            let actual = (state == PromiseState::Rejected).then_some(val);
            let checked = if want_reject {
                expects_error("rejects", "rejection", actual, &rest)
            } else {
                expects_no_error("doesNotReject", "rejection", actual, &rest)
            };
            match checked {
                Ok(()) => resolve_promise_val(rid, Value::Undef),
                Err(e) => {
                    let ev = take_exc_or_error(&e);
                    reject_promise_val(rid, ev);
                }
            }
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
    assertion_error_object(
        &msg,
        generated,
        operator.as_deref(),
        actual.unwrap_or(Value::Undef),
        expected.unwrap_or(Value::Undef),
    )
}

/// Node's `diff` field. Every failure form measured on node v26.7.0 — `ok`,
/// `equal`, `strictEqual`, `notStrictEqual`, `deepEqual`, `deepStrictEqual`,
/// `match`, `throws`, `fail` — reports the same `"simple"`; it names the diff
/// MODE the error was built under, not a rendered diff.
const DIFF_MODE: &str = "simple";

/// Build the `AssertionError` object a failing assertion throws.
///
/// The own-property set is what a test runner reads, and node-js carried only
/// `code`/`message`/`stack`: `err.actual`, `err.expected`, `err.operator` and
/// `err.generatedMessage` were all `undefined`, so every framework that reports
/// "expected X, got Y" from a caught `AssertionError` had nothing to report.
/// Measured on node v26.7.0, `Object.keys(err)` is
/// `["generatedMessage","code","actual","expected","operator","diff"]` — in that
/// order — while `name`, `message` and `stack` are own but NOT enumerable.
fn assertion_error_object(
    msg: &str,
    generated: bool,
    operator: Option<&str>,
    actual: Value,
    expected: Value,
) -> Value {
    let stack = format!("AssertionError [ERR_ASSERTION]: {msg}\n    at <anonymous>");
    let op_val = match operator {
        Some(o) => with_host(|h| h.new_str(o)),
        None => Value::Undef,
    };
    let name_v = with_host(|h| h.new_str("AssertionError"));
    let msg_v = with_host(|h| h.new_str(msg));
    let code_v = with_host(|h| h.new_str("ERR_ASSERTION"));
    let stack_v = with_host(|h| h.new_str(stack));
    let diff_v = with_host(|h| h.new_str(DIFF_MODE));
    let mut props: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
    // Enumerable, in node's order, first.
    props.insert("generatedMessage".into(), Value::Bool(generated));
    props.insert("code".into(), code_v);
    props.insert("actual".into(), actual);
    props.insert("expected".into(), expected);
    props.insert("operator".into(), op_val);
    props.insert("diff".into(), diff_v);
    props.insert("name".into(), name_v);
    props.insert("message".into(), msg_v);
    props.insert("stack".into(), stack_v);
    let obj = with_host(|h| h.new_object(props));
    with_host(|h| {
        for k in ["name", "message", "stack"] {
            h.hide_prop(&obj, k);
        }
        h.ensure_error_protos();
        // The `AssertionError` prototype, not `Error`'s: `e.constructor.name`
        // is what a test runner branches on, and linking straight to `Error`
        // reported `Error` there while `e.name` still said `AssertionError`.
        if let Some(p) = crate::host::error_proto_of(h, "AssertionError") {
            h.set_proto(&obj, p);
        }
    });
    obj
}

/// Raise a failing assertion as a REAL `AssertionError` object.
///
/// The internal `Name [CODE]: message` string is still what propagates (it is
/// what an uncaught failure prints), but the live thrown VALUE is parked in
/// `host.exc` so a `catch` receives the object with its full property set rather
/// than one synthesized from the message alone.
fn throw_assertion(
    msg: &str,
    generated: bool,
    operator: &str,
    actual: Value,
    expected: Value,
) -> String {
    let err = assertion_error_object(msg, generated, Some(operator), actual, expected);
    with_host(|h| h.exc = Some(err));
    assertion_error(msg)
}

/// `assert(value[, message])` — throws unless `value` is truthy.
pub fn assert_ok(args: &[Value]) -> Result<Value, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    if with_host(|h| h.truthy(&v)) {
        return Ok(Value::Undef);
    }
    let custom = message(args, 1);
    let msg = custom.clone().unwrap_or_else(||
        // Node's heading ends with a colon and is followed by an echo of
        // the failing source line, which needs the call site's text.
        "The expression evaluated to a falsy value:".to_string());
    // `ok` reports the operand as `actual` against a literal `true`, under the
    // `==` operator (measured on node v26.7.0: `assert.ok(0)` gives
    // `actual: 0`, `expected: true`, `operator: '=='`).
    Err(throw_assertion(
        &msg,
        custom.is_none(),
        "==",
        v,
        Value::Bool(true),
    ))
}

fn check(
    pass: bool,
    args: &[Value],
    msg_idx: usize,
    op: &str,
    a: &Value,
    b: &Value,
) -> Result<Value, String> {
    if pass {
        return Ok(Value::Undef);
    }
    let custom = message(args, msg_idx);
    // `strictEqual`, `deepStrictEqual` and `partialDeepStrictEqual` are Node's
    // `kMethodsWithCustomMessageDiff`: they render a structural `+ actual -
    // expected` diff of the two operands, and they keep rendering it when a
    // custom message is supplied (the custom text replaces the heading only).
    // Every other comparison writes a fixed sentence.
    let diff_operator = match op {
        "===" => Some("strictEqual"),
        "deepStrictEqual" => Some("deepStrictEqual"),
        "partialDeepStrictEqual" => Some("partialDeepStrictEqual"),
        _ => None,
    };
    if let Some(diff_op) = diff_operator {
        let msg = super::assert_diff::create_err_diff(a, b, diff_op, custom.as_deref());
        let operator = if op == "===" { "strictEqual" } else { op };
        return Err(throw_assertion(
            &msg,
            custom.is_none(),
            operator,
            a.clone(),
            b.clone(),
        ));
    }
    // The remaining messages echo one or both operands, and do so with assert's
    // OWN inspect settings (expanded and sorted), not `console.log`'s: node
    // reports `notDeepStrictEqual({a:1},{a:1})` as `{\n  a: 1\n}`, one property
    // per line, where the default rendering is `{ a: 1 }`.
    let (sa, sb) = (
        super::assert_diff::inspect_operand(a),
        super::assert_diff::inspect_operand(b),
    );
    // Each comparison has its OWN generated-message shape in Node; `{a} {op} {b}`
    // is only right for the two loose forms. `strictEqual(1, 2)` produced
    // `1 strictEqual 2` here, which is not a sentence any Node emits — the
    // operator name was being substituted where Node writes a whole heading.
    let msg = match op {
        "==" | "!=" => format!("{sa} {op} {sb}"),
        "===" => format!("Expected values to be strictly equal:\n\n{sa} !== {sb}\n"),
        "!==" => format!("Expected \"actual\" to be strictly unequal to: {sa}"),
        "deepEqual" => format!(
            "Expected values to be loosely deep-equal:\n\n{sa}\n\nshould loosely \
             deep-equal\n\n{sb}"
        ),
        "notDeepEqual" => {
            format!("Expected \"actual\" not to be loosely deep-equal to:\n\n{sa}")
        }
        // The `+ actual - expected` structural DIFF Node renders between two
        // non-primitive operands is not reproduced (it needs a line-oriented
        // differ over `util.inspect` output); the primitive form, which is the
        // whole message when neither side is an object, is exact.
        "deepStrictEqual" => {
            format!("Expected values to be strictly deep-equal:\n\n{sa} !== {sb}\n")
        }
        "notDeepStrictEqual" => {
            format!("Expected \"actual\" not to be strictly deep-equal to:\n\n{sa}\n")
        }
        _ => format!("{sa} {op} {sb}"),
    };
    // Node names the METHOD for the strict/deep forms and the OPERATOR for the
    // two loose ones: `strictEqual` reports `operator: 'strictEqual'` while
    // `equal` reports `'=='`. A custom message replaces the generated text but
    // keeps every other field, and flips `generatedMessage` to false.
    let operator = match op {
        "===" => "strictEqual",
        "!==" => "notStrictEqual",
        other => other,
    };
    Err(throw_assertion(
        &custom.clone().unwrap_or(msg),
        custom.is_none(),
        operator,
        a.clone(),
        b.clone(),
    ))
}

/// `assert.throws(fn[, error][, message])` / `assert.doesNotThrow(...)`: a port
/// of node's `expectsError`/`expectedException` and `expectsNoError`
/// (`lib/assert.js`). The `error` argument was ignored entirely, so
/// `assert.throws(fn, RangeError)` passed whatever `fn` threw.
fn throws(args: &[Value], want_throw: bool) -> Result<Value, String> {
    let f = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| is_callable(h, &f)) {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"fn\" argument must be of type function. Received {}",
                crate::stdlib::received_desc(&f)
            ),
        ));
    }
    // The thrown value becomes `err.actual`, so it has to be captured rather
    // than discarded with `.is_err()`.
    let caught = match invoke(&f, Vec::new(), None) {
        Ok(_) => None,
        Err(e) => Some(crate::host::take_exc_or_error(&e)),
    };
    let rest = args.get(1..).unwrap_or(&[]);
    if want_throw {
        expects_error("throws", "exception", caught, rest)
    } else {
        expects_no_error("doesNotThrow", "exception", caught, rest)
    }
    .map(|_| Value::Undef)
}

/// The `error` argument's validation (`expectsError`). `actual` is `None`
/// when nothing was thrown (or rejected).
fn expects_error(
    operator: &str,
    kind: &str,
    actual: Option<Value>,
    rest: &[Value],
) -> Result<(), String> {
    let mut error = rest.first().cloned().unwrap_or(Value::Undef);
    let mut msg = message(rest, 1);
    let type_of = |v: &Value| with_host(|h| h.type_of(v));
    if type_of(&error) == "string" {
        if rest.len() >= 2 {
            return Err(invalid_error_arg(&error));
        }
        let text = with_host(|h| h.str_of(&error));
        if let Some(a) = &actual {
            if is_object(a) {
                let m = crate::builtins::get_property(a, "message")?;
                if with_host(|h| h.type_of(&m) == "string" && h.str_of(&m) == text) {
                    return Err(ambiguous(&format!(
                        "The error message \"{text}\" is identical to the message."
                    )));
                }
            } else if with_host(|h| h.type_of(a) == "string" && h.str_of(a) == text) {
                return Err(ambiguous(&format!(
                    "The error \"{text}\" is identical to the message."
                )));
            }
        }
        msg = Some(text);
        error = Value::Undef;
    } else if !with_host(|h| h.is_nullish(&error))
        && !matches!(type_of(&error), "object" | "function")
    {
        return Err(invalid_error_arg(&error));
    }
    let Some(actual) = actual else {
        let name = if with_host(|h| h.is_nullish(&error)) {
            None
        } else {
            let n = crate::builtins::get_property(&error, "name")?;
            with_host(|h| h.truthy(&n).then(|| h.str_of(&n)))
        };
        let mut details = name.map(|n| format!(" ({n})")).unwrap_or_default();
        details.push_str(&msg.map(|m| format!(": {m}")).unwrap_or_else(|| ".".into()));
        return Err(throw_assertion(
            &format!("Missing expected {kind}{details}"),
            false,
            operator,
            Value::Undef,
            error,
        ));
    };
    if !with_host(|h| h.truthy(&error)) {
        return Ok(());
    }
    expected_exception(operator, actual, error, msg)
}

fn invalid_error_arg(error: &Value) -> String {
    crate::host::coded_error(
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"error\" argument must be of type function or an instance of Error, RegExp, or Object. Received {}",
            crate::stdlib::received_desc(error)
        ),
    )
}

fn ambiguous(detail: &str) -> String {
    crate::host::coded_error(
        "TypeError",
        "ERR_AMBIGUOUS_ARGUMENT",
        &format!("The \"error/message\" argument is ambiguous. {detail}"),
    )
}

fn is_object(v: &Value) -> bool {
    with_host(|h| h.type_of(v) == "object" && !h.is_null(v))
}

/// Whether `v` is an Error (node's `isError`: its brand is `Error`).
fn is_error(v: &Value) -> bool {
    is_object(v) && with_host(|h| crate::builtins::object_tag(h, v) == "[object Error]")
}

/// `expectedException`: check what was thrown against a RegExp, a validation
/// object, an Error class or a validation function.
fn expected_exception(
    operator: &str,
    actual: Value,
    expected: Value,
    msg: Option<String>,
) -> Result<(), String> {
    let generated = msg.is_none();
    let fail = |text: String| -> Result<(), String> {
        Err(throw_assertion(
            &text,
            generated,
            operator,
            actual.clone(),
            expected.clone(),
        ))
    };
    if with_host(|h| h.type_of(&expected)) != "function" {
        if is_regexp(&expected) {
            let s = crate::host::to_string_value(&actual)?;
            if regexp_matches(&expected, &s)? {
                return Ok(());
            }
            return fail(msg.unwrap_or_else(|| {
                with_host(|h| {
                    format!(
                        "The input did not match the regular expression {}. Input:\n\n{}\n",
                        h.inspect(&expected),
                        h.inspect(&s)
                    )
                })
            }));
        }
        if !is_object(&actual) {
            let text = msg.unwrap_or_else(|| {
                super::assert_diff::create_err_diff(&actual, &expected, "deepStrictEqual", None)
            });
            return fail(text);
        }
        // A validation object: every key must be deep-strict-equal, or a
        // RegExp matching a string property. An Error compares `name` and
        // `message` as well.
        let keys_v = crate::builtins::call_builtin_function("Object.keys", vec![expected.clone()])?;
        let mut keys: Vec<Value> = with_host(|h| match h.get(&keys_v) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        });
        if is_error(&expected) {
            keys.extend(with_host(|h| [h.new_str("name"), h.new_str("message")]));
        } else if keys.is_empty() {
            let shown = with_host(|h| h.inspect(&expected));
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                &format!("The argument 'error' may not be an empty object. Received {shown}"),
            ));
        }
        for key in &keys {
            let k = with_host(|h| h.str_of(key));
            let a = crate::builtins::get_property(&actual, &k)?;
            let e = crate::builtins::get_property(&expected, &k)?;
            if with_host(|h| h.type_of(&a)) == "string" && is_regexp(&e) && regexp_matches(&e, &a)?
            {
                continue;
            }
            let present = crate::builtins::has_property(&actual, &k)?;
            if present && deep_equal(&a, &e, true) {
                continue;
            }
            let text = match &msg {
                Some(m) => m.clone(),
                None => {
                    let keys_arr = with_host(|h| h.new_array(keys.clone()));
                    let ca = comparison(&actual, &keys_arr, &Value::Undef)?;
                    let cb = comparison(&expected, &keys_arr, &actual)?;
                    super::assert_diff::create_err_diff(&ca, &cb, "deepStrictEqual", None)
                }
            };
            return fail(text);
        }
        return Ok(());
    }
    // An Error class: the thrown value must be an instance of it.
    let proto = crate::builtins::get_property(&expected, "prototype")?;
    if !matches!(proto, Value::Undef) && crate::host::instance_of(&actual, &expected)? {
        return Ok(());
    }
    if is_error_subclass(&expected) {
        let text = match msg {
            Some(m) => m,
            None => {
                let name = with_host(|h| h.callable_name(&expected));
                let mut m =
                    format!("The error is expected to be an instance of \"{name}\". Received ");
                if is_error(&actual) {
                    let ctor = crate::builtins::get_property(&actual, "constructor")?;
                    let ctor_name = if with_host(|h| h.truthy(&ctor)) {
                        crate::builtins::get_property(&ctor, "name")?
                    } else {
                        Value::Undef
                    };
                    let actual_name = if with_host(|h| h.truthy(&ctor_name)) {
                        with_host(|h| h.str_of(&ctor_name))
                    } else {
                        let n = crate::builtins::get_property(&actual, "name")?;
                        with_host(|h| h.str_of(&n))
                    };
                    if actual_name == name {
                        m.push_str("an error with identical name but a different prototype.");
                    } else {
                        m.push_str(&format!("\"{actual_name}\""));
                    }
                    let am = crate::builtins::get_property(&actual, "message")?;
                    if with_host(|h| h.truthy(&am)) {
                        m.push_str(&format!(
                            "\n\nError message:\n\n{}",
                            with_host(|h| h.str_of(&am))
                        ));
                    }
                } else {
                    m.push_str(&format!("\"{}\"", with_host(|h| h.inspect(&actual))));
                }
                m
            }
        };
        return fail(text);
    }
    // A validation function, called with a fresh `this`, must return `true`.
    let this = with_host(|h| h.new_object(indexmap::IndexMap::new()));
    let res = invoke(&expected, vec![actual.clone()], Some(this))?;
    if matches!(res, Value::Bool(true)) {
        return Ok(());
    }
    let text = match msg {
        Some(m) => m,
        None => {
            let name = with_host(|h| h.callable_name(&expected));
            let named = if name.is_empty() {
                String::new()
            } else {
                format!("\"{name}\" ")
            };
            let mut m = format!(
                "The {named}validation function is expected to return \"true\". Received {}",
                with_host(|h| h.inspect(&res))
            );
            if is_error(&actual) {
                let s = crate::host::to_string_value(&actual)?;
                m.push_str(&format!(
                    "\n\nCaught error:\n\n{}",
                    with_host(|h| h.str_of(&s))
                ));
            }
            m
        }
    };
    fail(text)
}

/// `expectsNoError`: anything thrown fails, unless an `error` argument is
/// given that it does NOT match, in which case it is rethrown as is.
fn expects_no_error(
    operator: &str,
    kind: &str,
    actual: Option<Value>,
    rest: &[Value],
) -> Result<(), String> {
    let Some(actual) = actual else {
        return Ok(());
    };
    let mut error = rest.first().cloned().unwrap_or(Value::Undef);
    let mut msg = message(rest, 1);
    if with_host(|h| h.type_of(&error)) == "string" {
        msg = Some(with_host(|h| h.str_of(&error)));
        error = Value::Undef;
    }
    if !with_host(|h| h.truthy(&error)) || has_matching_error(&actual, &error)? {
        let details = msg.map(|m| format!(": {m}")).unwrap_or_else(|| ".".into());
        let actual_message = if is_object(&actual) {
            let m = crate::builtins::get_property(&actual, "message")?;
            with_host(|h| h.str_of(&m))
        } else {
            "undefined".to_string()
        };
        return Err(throw_assertion(
            &format!("Got unwanted {kind}{details}\nActual message: \"{actual_message}\""),
            false,
            operator,
            actual,
            error,
        ));
    }
    Err(with_host(|h| {
        h.exc = Some(actual.clone());
        crate::builtins::error_string(h, &actual)
    }))
}

/// `hasMatchingError`.
fn has_matching_error(actual: &Value, expected: &Value) -> Result<bool, String> {
    if with_host(|h| h.type_of(expected)) != "function" {
        if is_regexp(expected) {
            let s = crate::host::to_string_value(actual)?;
            return regexp_matches(expected, &s);
        }
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"expected\" argument must be of type function or an instance of RegExp. Received {}",
                crate::stdlib::received_desc(expected)
            ),
        ));
    }
    let proto = crate::builtins::get_property(expected, "prototype")?;
    if !matches!(proto, Value::Undef) && crate::host::instance_of(actual, expected)? {
        return Ok(true);
    }
    if is_error_subclass(expected) {
        return Ok(false);
    }
    let this = with_host(|h| h.new_object(indexmap::IndexMap::new()));
    let res = invoke(expected, vec![actual.clone()], Some(this))?;
    Ok(matches!(res, Value::Bool(true)))
}

/// `ObjectPrototypeIsPrototypeOf(Error, expected)`: whether `Error` is on
/// `expected`'s prototype chain — a subclass of it, not `Error` itself.
fn is_error_subclass(expected: &Value) -> bool {
    let mut cur = crate::builtins::prototype_of(expected);
    for _ in 0..100_000 {
        if with_host(|h| h.is_null(&cur) || matches!(cur, Value::Undef)) {
            return false;
        }
        if with_host(|h| matches!(h.get(&cur), Some(JsObj::Builtin(n)) if n == "Error")) {
            return true;
        }
        cur = crate::builtins::prototype_of(&cur);
    }
    false
}

/// Node's `Comparison`: an object of the compared keys, so the diff an
/// `assert.throws` validation object fails with names only those keys, under
/// the class name `Comparison`. Defined once, in JavaScript, since it IS a
/// class node defines in JavaScript.
fn comparison(obj: &Value, keys: &Value, actual: &Value) -> Result<Value, String> {
    thread_local! {
        static FACTORY: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }
    let factory = match FACTORY.with(|f| f.borrow().clone()) {
        Some(f) => f,
        None => {
            let f = crate::eval_in_global_scope(
                "(() => { class Comparison { constructor(obj, keys, actual) { for (const key of keys) { if (key in obj) { if (actual !== undefined && typeof actual[key] === 'string' && Object.prototype.toString.call(obj[key]) === '[object RegExp]' && obj[key].exec(actual[key]) !== null) { this[key] = actual[key]; } else { this[key] = obj[key]; } } } } } return (obj, keys, actual) => new Comparison(obj, keys, actual); })()",
            )?;
            FACTORY.with(|c| *c.borrow_mut() = Some(f.clone()));
            f
        }
    };
    invoke(
        &factory,
        vec![obj.clone(), keys.clone(), actual.clone()],
        None,
    )
}

fn message(args: &[Value], idx: usize) -> Option<String> {
    match args.get(idx) {
        Some(Value::Undef) | None => None,
        Some(v) => Some(with_host(|h| h.str_of(v))),
    }
}

/// An `AssertionError` as an internal error string.
///
/// `Name [CODE]: message` is the shared encoding `synth_error` parses back into
/// `.name`/`.code`/`.message`, so the prefix must be built by the one
/// constructor rather than written out here — spelling it inline is what let
/// this site keep a form the parser did not recognize.
fn assertion_error(msg: &str) -> String {
    crate::host::coded_error("AssertionError", "ERR_ASSERTION", msg)
}

/// `assert.strictEqual` compares with `Object.is`, not `===`.
///
/// That is the whole difference for two values: `NaN` equals itself, and `+0`
/// does not equal `-0`. Using `===` had both backwards — `strictEqual(NaN, NaN)`
/// failed and `strictEqual(0, -0)` passed.
fn strict(a: &Value, b: &Value) -> bool {
    crate::builtins::same_value(a, b)
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
    deep_equal_seen(a, b, strict_mode, &mut Vec::new())
}

/// `deep_equal` carrying the pairs currently being compared.
///
/// Without it a self-referential structure recursed until the stack overflowed
/// and the process aborted — `const x = {}; x.self = x;` compared against
/// another of the same shape, which is exactly what a test asserting on a
/// linked structure does. A pair already on the stack is treated as equal: if
/// anything else about the two differs, some other comparison finds it.
fn deep_equal_seen(
    a: &Value,
    b: &Value,
    strict_mode: bool,
    seen: &mut Vec<(Value, Value)>,
) -> bool {
    if seen.iter().any(|(x, y)| x == a && y == b) {
        return true;
    }
    // `deepStrictEqual` requires the two to share a [[Prototype]]. That single
    // check is what separates `Object.create(null)` from `{}`, an instance of
    // one class from an instance of another, and a `Uint8Array` from an
    // `Int8Array` — none of which were being distinguished.
    if strict_mode {
        let both_objects = with_host(|h| h.get(a).is_some() && h.get(b).is_some());
        if both_objects
            && with_host(|h| {
                // A null-prototype object is tracked separately rather than by
                // `proto_of` returning None — which a plain object does too, its
                // `Object.prototype` being implicit. Comparing only `proto_of`
                // therefore called `Object.create(null)` and `{}` alike.
                h.proto_of(a) != h.proto_of(b) || h.has_null_proto(a) != h.has_null_proto(b)
            })
        {
            return false;
        }
    }
    let kinds = with_host(|h| {
        let av = h.get(a).map(kind);
        let bv = h.get(b).map(kind);
        (av, bv)
    });
    seen.push((a.clone(), b.clone()));
    let result = deep_equal_body(a, b, strict_mode, seen, kinds);
    seen.pop();
    result
}

fn deep_equal_body(
    a: &Value,
    b: &Value,
    strict_mode: bool,
    seen: &mut Vec<(Value, Value)>,
    kinds: (Option<Kind>, Option<Kind>),
) -> bool {
    match kinds {
        (Some(Kind::Array), Some(Kind::Array)) => {
            let (ia, ib) = with_host(|h| (array_of(h, a), array_of(h, b)));
            ia.len() == ib.len()
                && ia
                    .iter()
                    .zip(ib.iter())
                    .all(|(x, y)| deep_equal_seen(x, y, strict_mode, seen))
        }
        (Some(Kind::Object), Some(Kind::Object)) => {
            let (ea, eb) = with_host(|h| (object_of(h, a), object_of(h, b)));
            if ea.len() != eb.len() {
                return false;
            }
            let props_match = ea.iter().all(|(k, va)| {
                eb.iter()
                    .find(|(k2, _)| k2 == k)
                    .is_some_and(|(_, vb)| deep_equal_seen(va, vb, strict_mode, seen))
            });
            if !props_match {
                return false;
            }
            // The brands whose whole state lives in INTERNAL slots — a Date's
            // `@@ms`, a typed array's `@@buffer`/`byteOffset`, an Error's name
            // and message — are objects with no enumerable own properties at
            // all. Comparing only the public ones therefore reported every pair
            // of them as deep-equal: `deepStrictEqual(new Date(0), new Date(1))`
            // PASSED, as did two Buffers with different bytes. Slots are
            // compared here rather than folded into `object_of` because they are
            // not properties — they must not affect the key COUNT above, which
            // node takes over enumerable keys only.
            let (ia, ib) = with_host(|h| (internals_of(h, a), internals_of(h, b)));
            if ia.len() != ib.len() {
                return false;
            }
            ia.iter().all(|(k, va)| {
                ib.iter()
                    .find(|(k2, _)| k2 == k)
                    .is_some_and(|(_, vb)| deep_equal_seen(va, vb, strict_mode, seen))
            })
        }
        // A Map compares by ENTRIES and a Set by MEMBERS, both order-insensitively
        // (`new Set([1, 2])` deep-equals `new Set([2, 1])`), so each entry on the
        // left is matched against an as-yet-unclaimed entry on the right rather
        // than against the one at its own index.
        (Some(Kind::Map), Some(Kind::Map)) => {
            let (ea, eb) = with_host(|h| (map_entries_of(h, a), map_entries_of(h, b)));
            unordered_match(&ea, &eb, seen, |(ka, va), (kb, vb), seen| {
                deep_equal_seen(ka, kb, strict_mode, seen)
                    && deep_equal_seen(va, vb, strict_mode, seen)
            })
        }
        (Some(Kind::Set), Some(Kind::Set)) => {
            let (ea, eb) = with_host(|h| (set_members_of(h, a), set_members_of(h, b)));
            unordered_match(&ea, &eb, seen, |x, y, seen| {
                deep_equal_seen(x, y, strict_mode, seen)
            })
        }
        // Two distinct RegExp objects are deep-equal when their pattern and flags
        // are; `same_value` would call every one of them unequal.
        (Some(Kind::RegExp), Some(Kind::RegExp)) => {
            with_host(|h| regexp_key(h, a) == regexp_key(h, b))
        }
        // Everything else — strings, symbols, bigints, functions, and any two
        // values of DIFFERENT kinds — is compared as a leaf. This arm used to be
        // unreachable for heap values because `kind` called them all `Object`,
        // and `object_of` then reported each as having zero properties, so any
        // two of them matched: `deepStrictEqual('abc', 'abd')` passed silently.
        _ => {
            if strict_mode {
                strict(a, b)
            } else {
                loose_eq(a, b)
            }
        }
    }
}

/// Whether every element of `ea` can be paired off with a DISTINCT element of
/// `eb` under `eq`. Greedy matching is enough here because the relation is an
/// equivalence: anything that matches a claimed element would have matched
/// whatever claimed it.
fn unordered_match<T>(
    ea: &[T],
    eb: &[T],
    seen: &mut Vec<(Value, Value)>,
    eq: impl Fn(&T, &T, &mut Vec<(Value, Value)>) -> bool,
) -> bool {
    if ea.len() != eb.len() {
        return false;
    }
    let mut claimed = vec![false; eb.len()];
    'outer: for x in ea {
        for (i, y) in eb.iter().enumerate() {
            if !claimed[i] && eq(x, y, seen) {
                claimed[i] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

enum Kind {
    Array,
    Object,
    Map,
    Set,
    RegExp,
    /// A leaf: compared with `===`, never structurally.
    Other,
}
fn kind(o: &JsObj) -> Kind {
    match o {
        JsObj::Array(_) => Kind::Array,
        JsObj::Object(_) => Kind::Object,
        // A WEAK collection exposes no entries, so there is nothing to compare
        // structurally; node treats two of them as equal only by identity.
        JsObj::Map { weak: false, .. } => Kind::Map,
        JsObj::Set { weak: false, .. } => Kind::Set,
        JsObj::RegExp(_) => Kind::RegExp,
        _ => Kind::Other,
    }
}
/// A Map's entries as `(key, value)` pairs, in insertion order.
fn map_entries_of(h: &crate::host::JsHost, v: &Value) -> Vec<(Value, Value)> {
    match h.get(v) {
        Some(JsObj::Map { entries, .. }) => entries.values().cloned().collect(),
        _ => Vec::new(),
    }
}
/// A Set's members, in insertion order.
fn set_members_of(h: &crate::host::JsHost, v: &Value) -> Vec<Value> {
    match h.get(v) {
        Some(JsObj::Set { entries, .. }) => entries.values().cloned().collect(),
        _ => Vec::new(),
    }
}
/// The identity of a regular expression for comparison: its pattern and flags.
fn regexp_key(h: &crate::host::JsHost, v: &Value) -> Option<(String, String)> {
    match h.get(v) {
        Some(JsObj::RegExp(r)) => Some((r.source.clone(), r.flags.clone())),
        _ => None,
    }
}
/// An object's INTERNAL slots — the `@@`-prefixed keys that carry brand state
/// (`@@ms`, `@@buffer`, `@@kind`) and are deliberately absent from `object_of`.
/// Private class fields (`#`-prefixed) stay excluded: node's `deepStrictEqual`
/// compares own ENUMERABLE properties, and a private field is neither.
fn internals_of(h: &crate::host::JsHost, v: &Value) -> Vec<(String, Value)> {
    match h.get(v) {
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| k.starts_with("@@"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Vec::new(),
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
        Some(JsObj::Object(p)) => p
            .iter()
            .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Node's `checkIsPromise`: a native promise, or an object whose `then` and
/// `catch` are both functions.
fn is_promise_like(v: &Value) -> bool {
    if with_host(|h| h.promise_id(v)).is_some() {
        return true;
    }
    if !is_object(v) {
        return false;
    }
    ["then", "catch"].iter().all(|k| {
        crate::builtins::get_property(v, k)
            .map(|f| with_host(|h| is_callable(h, &f)))
            .unwrap_or(false)
    })
}
