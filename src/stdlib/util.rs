//! Node `util` module: `format`, `inspect`, and a subset of `util.types`.

use crate::host::{with_host, JsObj};
use fusevm::Value;

pub const METHODS: &[&str] = &[
    "format",
    "formatWithOptions",
    "inspect",
    "deprecate",
    "inherits",
    "types",
    "types.isMap",
    "types.isSet",
    "types.isPromise",
    "types.isDate",
    "types.isRegExp",
    "types.isNativeError",
    "types.isAsyncFunction",
    "isDeepStrictEqual",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if let Some(pred) = method.strip_prefix("types.") {
        return Some(Ok(Value::Bool(type_predicate(pred, args.first()))));
    }
    Some(match method {
        "format" => {
            // `format` re-enters `with_host` internally, so build the string first
            // and only then allocate — never nest the borrow.
            let s = format(args);
            Ok(with_host(|h| h.new_str(s)))
        }
        // `formatWithOptions(inspectOptions, fmt, ...args)`: the options object
        // only tunes `inspect` styling, which we do not vary — drop it and format
        // the remaining arguments exactly like `format`.
        "formatWithOptions" => {
            let s = format(args.get(1..).unwrap_or(&[]));
            Ok(with_host(|h| h.new_str(s)))
        }
        "inspect" => Ok(with_host(|h| {
            let s = h.inspect(&args.first().cloned().unwrap_or(Value::Undef));
            h.new_str(s)
        })),
        // `deprecate(fn, msg)`: return a callable that behaves like `fn`. The
        // house rule is no deprecation nags, so no warning is emitted — the
        // original function is handed back unchanged.
        "deprecate" => Ok(args.first().cloned().unwrap_or(Value::Undef)),
        // `inherits(ctor, superCtor)`: modern Node semantics — set `ctor.super_`
        // and re-link `ctor.prototype`'s `[[Prototype]]` to `superCtor.prototype`
        // (methods already on `ctor.prototype` are preserved).
        "inherits" => Ok(inherits(args)),
        // Bare `util.types` accessed then called is not meaningful; return the
        // namespace value so a stray call is a harmless undefined.
        "types" => Ok(Value::Undef),
        "isDeepStrictEqual" => Ok(Value::Bool(super::assert::deep_equal(
            &args.first().cloned().unwrap_or(Value::Undef),
            &args.get(1).cloned().unwrap_or(Value::Undef),
            true,
        ))),
        _ => return None,
    })
}

/// `util.inherits(ctor, superCtor)`.
fn inherits(args: &[Value]) -> Value {
    let ctor = args.first().cloned().unwrap_or(Value::Undef);
    let sup = args.get(1).cloned().unwrap_or(Value::Undef);
    with_host(|h| {
        // Get-or-create `superCtor.prototype` and store it, so a later
        // `superCtor.prototype` read returns the same object identity (`===`).
        let sup_proto = h.fn_prop(&sup, "prototype").unwrap_or_else(|| {
            let mut props = indexmap::IndexMap::new();
            props.insert("constructor".to_string(), sup.clone());
            let p = h.new_object(props);
            h.set_fn_prop(&sup, "prototype", p.clone());
            p
        });
        // Get-or-create `ctor.prototype` (with a `constructor` back-link).
        let ctor_proto = h.fn_prop(&ctor, "prototype").unwrap_or_else(|| {
            let mut props = indexmap::IndexMap::new();
            props.insert("constructor".to_string(), ctor.clone());
            let p = h.new_object(props);
            h.set_fn_prop(&ctor, "prototype", p.clone());
            p
        });
        h.set_proto(&ctor_proto, sup_proto);
        h.set_fn_prop(&ctor, "super_", sup);
    });
    Value::Undef
}

fn type_predicate(pred: &str, v: Option<&Value>) -> bool {
    let Some(v) = v else { return false };
    with_host(|h| match pred {
        "isMap" => matches!(h.get(v), Some(JsObj::Map { weak: false, .. })),
        "isSet" => matches!(h.get(v), Some(JsObj::Set { weak: false, .. })),
        "isPromise" => matches!(h.get(v), Some(JsObj::Promise { .. })),
        _ => false,
    })
}

/// `util.format(fmt, ...args)` — printf-style substitution (`%s %d %i %f %j %o %O
/// %c %%`) with any leftover arguments appended space-separated.
pub fn format(args: &[Value]) -> String {
    if args.is_empty() {
        return String::new();
    }
    // Node: a single argument is returned as-is (no specifier processing) —
    // `util.format("100%% done")` === "100%% done".
    if args.len() == 1 {
        return with_host(|h| h.console_format(&args[0]));
    }
    let fmt = with_host(|h| h.str_of(&args[0]));
    // A non-string first argument: inspect everything, space-joined.
    if !matches!(args[0], Value::Str(_)) && !with_host(|h| matches!(h.get(&args[0]), Some(JsObj::Str(_)))) {
        return with_host(|h| args.iter().map(|a| h.console_format(a)).collect::<Vec<_>>().join(" "));
    }

    let mut out = String::new();
    let mut ai = 1usize;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&spec) = chars.peek() else {
            out.push('%');
            break;
        };
        if spec == '%' {
            out.push('%');
            chars.next();
            continue;
        }
        if !matches!(spec, 's' | 'd' | 'i' | 'f' | 'j' | 'o' | 'O' | 'c') || ai >= args.len() {
            out.push('%');
            continue;
        }
        chars.next();
        let arg = &args[ai];
        ai += 1;
        match spec {
            // Node's %s renders a BigInt with the trailing `n` (unlike String()).
            's' => {
                let s = with_host(|h| match h.get(arg) {
                    Some(JsObj::BigInt(b)) => format!("{b}n"),
                    _ => h.str_of(arg),
                });
                out.push_str(&s);
            }
            'd' | 'i' => {
                let n = with_host(|h| h.to_number(arg));
                out.push_str(&if n.is_nan() { "NaN".into() } else { (n.trunc() as i64).to_string() });
            }
            'f' => out.push_str(&crate::host::fmt_number(with_host(|h| h.to_number(arg)))),
            'j' => {
                let s = crate::builtins::call_builtin_function("JSON.stringify", vec![arg.clone()])
                    .ok()
                    .map(|v| with_host(|h| h.str_of(&v)))
                    .unwrap_or_else(|| "undefined".into());
                out.push_str(&s);
            }
            'o' | 'O' => out.push_str(&with_host(|h| h.inspect(arg))),
            'c' => {} // CSS directive: consumes the arg, emits nothing.
            _ => {}
        }
    }
    // Append remaining arguments.
    for a in &args[ai..] {
        out.push(' ');
        out.push_str(&with_host(|h| h.console_format(a)));
    }
    out
}
