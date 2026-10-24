//! Node `util` module: `format`, `inspect`, and a subset of `util.types`.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

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
    "isArray",
    "debuglog",
    "stripVTControlCharacters",
    "toUSVString",
    "getSystemErrorName",
    "getSystemErrorMessage",
    "getSystemErrorMap",
    "styleText",
    "parseArgs",
    "promisify",
    "aborted",
    "callbackify",
    "parseEnv",
    "debug",
];

/// Non-function `util` exports (`require('util').TextEncoder`, `.MIMEType`, …).
/// Each resolves to a `Builtin("<name>")` the parent `construct`s via `new`.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "types" => Some(with_host(|h| h.alloc(JsObj::Builtin("util/types".into())))),
        "TextEncoder" | "TextDecoder" | "MIMEType" | "MIMEParams" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        _ => None,
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if let Some(pred) = method.strip_prefix("types.") {
        return Some(Ok(Value::Bool(type_predicate(pred, args.first()))));
    }
    Some(match method {
        "format" => {
            // `format` re-enters `with_host` internally, so build the string first
            // and only then allocate — never nest the borrow.
            match format(args) {
                Ok(s) => Ok(with_host(|h| h.new_str(s))),
                Err(e) => Err(e),
            }
        }
        // `formatWithOptions(inspectOptions, fmt, ...args)`: the options object
        // only tunes `inspect` styling, which we do not vary — drop it and format
        // the remaining arguments exactly like `format`.
        "formatWithOptions" => match format(args.get(1..).unwrap_or(&[])) {
            Ok(s) => Ok(with_host(|h| h.new_str(s))),
            Err(e) => Err(e),
        },
        "inspect" => {
            if let Some(v) = args.first() {
                crate::builtins::materialize_stack(v);
            }
            // Honor an options `{ depth: N | null }` (default 2; null = unlimited).
            let depth = match args.get(1) {
                Some(opts) => match crate::builtins::get_property(opts, "depth") {
                    Ok(Value::Undef) => 2,
                    Ok(v) if with_host(|h| h.is_null(&v)) => i64::MAX,
                    Ok(v) => {
                        let n = with_host(|h| h.to_number(&v));
                        // A NEGATIVE depth is legal and means "already past the
                        // limit"; only `null` and a non-finite depth mean
                        // unlimited. Treating anything below zero as unlimited
                        // made `{ depth: -1 }` print the whole object.
                        match n {
                            _ if n.is_nan() || n == f64::INFINITY => i64::MAX,
                            _ if n == f64::NEG_INFINITY => i64::MIN,
                            _ => n.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64,
                        }
                    }
                    Err(_) => 2,
                },
                None => 2,
            };
            crate::host::set_inspect_max_depth(depth);
            // `compact` (default 3) decides how deep a subtree may be and still
            // print on one line; `false` means never. `breakLength` (80 here) is
            // the column budget for that line.
            let opts = args.get(1).cloned().unwrap_or(Value::Undef);
            let read = |k: &str| crate::builtins::get_property(&opts, k).unwrap_or(Value::Undef);
            let compact = match read("compact") {
                Value::Undef => 3,
                Value::Bool(false) => 0,
                // `compact: true` is node's legacy "always join" mode; no depth
                // gate applies, which a very large number expresses exactly.
                Value::Bool(true) => i64::MAX,
                v => with_host(|h| h.to_number(&v)).max(0.0) as i64,
            };
            let break_length = match read("breakLength") {
                Value::Undef => 80,
                v => {
                    let n = with_host(|h| h.to_number(&v));
                    if n.is_finite() {
                        n.max(0.0) as usize
                    } else {
                        usize::MAX
                    }
                }
            };
            // `sorted` accepts a comparator function in node; only the boolean
            // form is honored here, and any truthy value turns plain sorting on.
            // Read the option OUT of the host before borrowing it: `read` borrows
            // the host itself, so `with_host(|h| h.truthy(&read("sorted")))` is a
            // nested borrow and aborts the process with "RefCell already
            // borrowed", which no script can catch.
            let sorted_opt = read("sorted");
            let sorted = with_host(|h| h.truthy(&sorted_opt));
            // `customInspect` defaults to TRUE, so only an explicit `false`
            // turns a value's own `[util.inspect.custom]` rendering off.
            let custom_inspect = match read("customInspect") {
                Value::Undef => true,
                v => with_host(|h| h.truthy(&v)),
            };
            let show_hidden_opt = read("showHidden");
            let show_hidden = with_host(|h| h.truthy(&show_hidden_opt));
            // `maxArrayLength: null | Infinity` means "no limit". Node also
            // accepts 0 (show none but the tail count), which falls out of the
            // clamp below without a special case.
            let max_array_length = match read("maxArrayLength") {
                Value::Undef => crate::host::DEFAULT_MAX_ARRAY_LENGTH,
                v if with_host(|h| h.is_null(&v)) => usize::MAX,
                v => {
                    let n = with_host(|h| h.to_number(&v));
                    if n.is_finite() {
                        n.max(0.0) as usize
                    } else {
                        usize::MAX
                    }
                }
            };
            let numeric_separator_opt = read("numericSeparator");
            let numeric_separator = with_host(|h| h.truthy(&numeric_separator_opt));
            let show_proxy_opt = read("showProxy");
            let show_proxy = with_host(|h| h.truthy(&show_proxy_opt));
            // `maxStringLength: null | Infinity` means "no limit", as for arrays.
            let max_string_length = match read("maxStringLength") {
                Value::Undef => crate::host::DEFAULT_MAX_STRING_LENGTH,
                v if with_host(|h| h.is_null(&v)) => usize::MAX,
                v => {
                    let n = with_host(|h| h.to_number(&v));
                    if n.is_finite() {
                        n.max(0.0) as usize
                    } else {
                        usize::MAX
                    }
                }
            };
            // `getters: true | 'get' | 'set'`.
            let getters_opt = read("getters");
            let getters = with_host(|h| match h.as_str(&getters_opt).as_deref() {
                Some("get") => 2,
                Some("set") => 3,
                _ if h.truthy(&getters_opt) => 1,
                _ => 0,
            });
            crate::host::set_inspect_getters(getters);
            crate::host::set_inspect_numeric_separator(numeric_separator);
            crate::host::set_inspect_show_proxy(show_proxy);
            crate::host::set_inspect_max_string_length(max_string_length);
            crate::host::set_inspect_compact(compact);
            crate::host::set_inspect_break_length(break_length);
            crate::host::set_inspect_sorted(sorted);
            crate::host::set_inspect_max_array_length(max_array_length);
            crate::host::set_inspect_custom(custom_inspect);
            crate::host::set_inspect_show_hidden(show_hidden);
            let rendered = crate::host::inspect_js(&args.first().cloned().unwrap_or(Value::Undef));
            let out = rendered.map(|s| with_host(|h| h.new_str(s)));
            crate::host::set_inspect_max_depth(2);
            crate::host::set_inspect_compact(3);
            crate::host::set_inspect_break_length(80);
            crate::host::set_inspect_sorted(false);
            crate::host::set_inspect_max_array_length(crate::host::DEFAULT_MAX_ARRAY_LENGTH);
            crate::host::set_inspect_custom(true);
            crate::host::set_inspect_getters(0);
            crate::host::set_inspect_numeric_separator(false);
            crate::host::set_inspect_show_proxy(false);
            crate::host::set_inspect_max_string_length(crate::host::DEFAULT_MAX_STRING_LENGTH);
            crate::host::set_inspect_show_hidden(false);
            out
        }
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
        // `util.isArray === Array.isArray` (a legacy alias Node still ships).
        "isArray" => Ok(Value::Bool(with_host(|h| {
            matches!(
                h.get(&args.first().cloned().unwrap_or(Value::Undef)),
                Some(JsObj::Array(_))
            )
        }))),
        // `stripVTControlCharacters(str)`: remove ANSI/VT escape sequences.
        "stripVTControlCharacters" => {
            let stripped = strip_vt(&super::arg_str(args, 0));
            Ok(with_host(|h| h.new_str(stripped)))
        }
        // `toUSVString(str)`: replace unpaired surrogates with U+FFFD. Strings are
        // already well-formed UTF-8 here (no lone surrogates survive), so the input
        // round-trips unchanged.
        "toUSVString" => Ok(with_host(|h| {
            let s = h.str_of(&args.first().cloned().unwrap_or(Value::Undef));
            h.new_str(s)
        })),
        "getSystemErrorName" => {
            let e = errno_of(super::arg_num(args, 0));
            let name = errno_name(e)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Unknown system error {e}"));
            Ok(with_host(|h| h.new_str(name)))
        }
        "getSystemErrorMessage" => {
            let e = errno_of(super::arg_num(args, 0));
            let msg = errno_message(e)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Unknown system error {e}"));
            Ok(with_host(|h| h.new_str(msg)))
        }
        "getSystemErrorMap" => Ok(system_error_map()),
        "styleText" => return Some(style_text(args)),
        "parseArgs" => return Some(parse_args(args)),
        "@@nodeError" => return Some(node_error(args)),
        // `getPromiseDetails(p)[0]` for the web-streams port's
        // `isPromisePending`: 0 pending, 1 fulfilled, 2 rejected.
        // The web-streams port's `readableStreamCancel(stream, reason)`, for
        // `FileHandle.readableWebStream`.
        "@@readableStreamCancel" => return Some(super::stream_web::readable_stream_cancel(args)),
        "@@promiseState" => {
            let p = args.first().cloned().unwrap_or(Value::Undef);
            let state = with_host(|h| h.promise_id(&p).map(|id| h.promise_state(id)));
            Ok(Value::Int(match state {
                Some(crate::host::PromiseState::Fulfilled) => 1,
                Some(crate::host::PromiseState::Rejected) => 2,
                _ => 0,
            }))
        }
        "debuglog" => return Some(debuglog(args)),
        "promisify" => return Some(promisify(args)),
        // `util.aborted(signal, resource)` — a promise that settles when the
        // signal aborts. It did not exist, so the documented way to await an
        // abort threw.
        "aborted" => {
            let signal = args.first().cloned().unwrap_or(Value::Undef);
            let resource = args.get(1).cloned().unwrap_or(Value::Undef);
            return Some(aborted(&signal, &resource));
        }
        "callbackify" => return Some(callbackify(args)),
        // `util.parseEnv(content)` → an object of the parsed dotenv assignments.
        "parseEnv" => Ok(parse_env(&super::arg_str(args, 0))),
        // `util.debug === util.debuglog` (a documented alias).
        "debug" => return Some(debuglog(args)),
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

/// Node's `formatNumber` for the `%d`/`%i`/`%f` directives: the ordinary number
/// rendering, except that negative zero prints as `-0` (plain string coercion
/// would flatten it to `0`, so `console.log("%d", -0)` would lose the sign).
fn fmt_directive_number(n: f64) -> String {
    if n == 0.0 && n.is_sign_negative() {
        return "-0".into();
    }
    crate::host::fmt_number(n)
}

/// Run one of the global numeric parsers over a directive argument. `%i` is
/// `parseInt` and `%f` is `parseFloat` — string-prefix parses, NOT `Number()`,
/// which is why `"3.9abc"` yields `3`/`3.9` rather than `NaN`.
fn coerce_via(parser: &str, arg: &Value) -> Result<f64, String> {
    // The parse itself cannot fail, but coercing the ARGUMENT to a string can:
    // `parseInt([Symbol()])` joins the array, and a Symbol has no string form.
    // Node lets that TypeError out of `util.format`; swallowing it turned a
    // throwing program into one that printed `NaN`.
    let v = crate::builtins::call_builtin_function(parser, vec![arg.clone()])?;
    Ok(with_host(|h| h.to_number(&v)))
}

/// `util.format(fmt, ...args)` — printf-style substitution (`%s %d %i %f %j %o %O
/// %c %%`) with any leftover arguments appended space-separated.
/// Fallible because three of the directives can throw and node lets those
/// throws out: `%j` on a BigInt is `TypeError: Do not know how to serialize a
/// BigInt`, and `%d`/`%i`/`%f` on a value whose coercion reaches a Symbol is
/// `TypeError: Cannot convert a Symbol value to a string`. Returning a bare
/// `String` meant each of those printed `undefined` or `NaN` instead.
pub fn format(args: &[Value]) -> Result<String, String> {
    // Node's inspect reads `err.stack`, and that read is what FORMATS the header
    // and freezes it. Doing it here, before the inspect borrow, keeps
    // `console.log(err)` showing the error's current `name` — the inspect walk
    // itself runs under an immutable host borrow and cannot re-format anything.
    for a in args {
        crate::builtins::materialize_stack(a);
    }
    if args.is_empty() {
        return Ok(String::new());
    }
    // Node: a single argument is returned as-is (no specifier processing) —
    // `util.format("100%% done")` === "100%% done".
    if args.len() == 1 {
        return crate::host::console_format_js(&args[0]);
    }
    let fmt = with_host(|h| h.str_of(&args[0]));
    // A non-string first argument: inspect everything, space-joined.
    if !matches!(args[0], Value::Str(_))
        && !with_host(|h| matches!(h.get(&args[0]), Some(JsObj::Str(_))))
    {
        return Ok(args
            .iter()
            .map(crate::host::console_format_js)
            .collect::<Result<Vec<_>, _>>()?
            .join(" "));
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
            // `%s` is NOT `String(x)` for objects. Node stringifies an object
            // only when its `toString` is one the SCRIPT wrote; an object that
            // would merely inherit the built-in is inspected instead, at
            // `depth: 0`. So `%s` on `{a:{b:1}}` is `{ a: [Object] }`, on a Date
            // the ISO form, and on a Map `Map(1) { 'a' => 1 }` — where plain
            // `String()` would flatten all three to `[object Object]`, which is
            // what this printed for every object, Map, Set, Date and boxed
            // primitive. A class that defines `toString` (or `Symbol
            // .toPrimitive`) still wins, at any depth of its prototype chain.
            's' => {
                // A NUMBER goes through node's number formatter, not `String()`:
                // that is the only path that distinguishes `-0` from `0`, and
                // `util.format('%s', -0)` is `-0` where `String(-0)` is `0`.
                if let Value::Float(n) = arg {
                    out.push_str(&fmt_directive_number(*n));
                    continue;
                }
                let (bigint, use_inspect) = with_host(|h| match h.get(arg) {
                    Some(JsObj::BigInt(b)) => (Some(format!("{b}n")), false),
                    _ => {
                        let is_obj = h.type_of(arg) == "object" && !h.is_null(arg);
                        // Only a SCRIPT-DEFINED conversion counts. A boxed
                        // primitive carries a real `toString` on its prototype,
                        // but it is the built-in one, so node still inspects it
                        // (`new String('b')` under `%s` is `[String: 'b']`, not
                        // `b`) — testing merely for the property's presence put
                        // every boxed primitive on the stringifying path.
                        // A Buffer is the one core type whose `toString` node
                        // does NOT count as built-in: `lib/buffer.js` is ordinary
                        // JavaScript, so `%s` decodes the bytes
                        // (`util.format('%s', Buffer.from('hi'))` is `hi`) rather
                        // than inspecting them. Its `toString` is native here and
                        // would otherwise be classified the other way. Read from
                        // the ALREADY-BORROWED host: `stdlib::native_tag` opens
                        // its own borrow and would abort the process from here.
                        // The same applies to every BYTE VIEW: node stringifies
                        // a `Uint8Array` under `%s` as `1,2`, where a plain Array
                        // is inspected as `[ 1, 2 ]`. An ArrayBuffer is NOT one of
                        // these — it has no `toString` of its own and is inspected.
                        let stringifies = matches!(h.get(arg), Some(JsObj::Object(p))
                        if matches!(
                            p.get("@@native").map(|t| h.str_of(t)).as_deref(),
                            Some("Buffer") | Some("TypedArray")
                        ));
                        if stringifies {
                            return (None, false);
                        }
                        let scripted = ["toString", "@@toPrimitive"].iter().any(|k| {
                            crate::host::lookup_chain(h, arg, k).is_some_and(|f| {
                                matches!(
                                    h.get(&f),
                                    Some(JsObj::Func(_))
                                        | Some(JsObj::Class(_))
                                        | Some(JsObj::BoundFunc { .. })
                                )
                            })
                        });
                        (None, is_obj && !scripted)
                    }
                });
                if let Some(b) = bigint {
                    out.push_str(&b);
                } else if use_inspect {
                    crate::host::set_inspect_max_depth(0);
                    let s = crate::host::inspect_js(arg);
                    crate::host::set_inspect_max_depth(2);
                    let s = s?;
                    out.push_str(&s);
                } else {
                    // Full `ToPrimitive`, so a scripted `toString` actually runs;
                    // `str_of` reads the object's brand and would answer
                    // `[object Object]` without calling anything.
                    let s = crate::host::to_string_value(arg)
                        .map(|v| with_host(|h| h.str_of(&v)))
                        .unwrap_or_else(|_| with_host(|h| h.str_of(arg)));
                    out.push_str(&s);
                }
            }
            // The three numeric directives use three DIFFERENT conversions, and
            // collapsing them to one truncating `to_number` got all three wrong:
            //   %d -> Number(x)      ("%d", 1.7)      is 1.7,      not 1
            //   %i -> parseInt(x,10) ("%i", "3.9abc") is 3,        not NaN
            //   %f -> parseFloat(x)  ("%f", "1.5x")   is 1.5,      not NaN
            // and `n.trunc() as i64` additionally saturated every non-finite and
            // out-of-i64-range value to 9223372036854775807 (`Infinity`, `1e21`).
            // BigInt keeps its `n` suffix under %d/%i (Node prints `10n`).
            'd' | 'i' => {
                let big = with_host(|h| match h.get(arg) {
                    Some(JsObj::BigInt(b)) => Some(format!("{b}n")),
                    _ => None,
                });
                // A bare Symbol is `NaN` under %d rather than a throw — node
                // tests `typeof === 'symbol'` before it ever coerces. Only a
                // Symbol reached THROUGH a coercion (inside an array, say) throws.
                let is_symbol = with_host(|h| h.type_of(arg) == "symbol");
                match big {
                    Some(s) => out.push_str(&s),
                    None if is_symbol => out.push_str("NaN"),
                    None if spec == 'd' => {
                        out.push_str(&fmt_directive_number(crate::host::to_number_value(arg)?))
                    }
                    None => out.push_str(&fmt_directive_number(coerce_via("parseInt", arg)?)),
                }
            }
            'f' => {
                let is_symbol = with_host(|h| h.type_of(arg) == "symbol");
                if is_symbol {
                    out.push_str("NaN");
                } else {
                    out.push_str(&fmt_directive_number(coerce_via("parseFloat", arg)?));
                }
            }
            'j' => {
                // A CIRCULAR structure is the one JSON failure node absorbs,
                // rendering it as `[Circular]`; every other failure — a BigInt
                // above all — propagates.
                match crate::builtins::call_builtin_function("JSON.stringify", vec![arg.clone()]) {
                    Ok(v) => out.push_str(&with_host(|h| h.str_of(&v))),
                    Err(e) if e.contains("circular structure") => out.push_str("[Circular]"),
                    Err(e) => return Err(e),
                }
            }
            // `%o` and `%O` are NOT the same directive. `%O` is a plain
            // `inspect` at the default depth; `%o` is `inspect(v, { showHidden:
            // true, showProxy: true, depth: 4 })`, so it reveals an array's
            // `[length]`, a typed array's window onto its buffer, and four levels
            // instead of two. Rendering both as the default inspect made `%o` a
            // silent alias of `%O`.
            'O' => out.push_str(&crate::host::inspect_js(arg)?),
            'o' => {
                crate::host::set_inspect_show_proxy(true);
                crate::host::set_inspect_show_hidden(true);
                crate::host::set_inspect_max_depth(4);
                let s = crate::host::inspect_js(arg);
                crate::host::set_inspect_show_hidden(false);
                crate::host::set_inspect_show_proxy(false);
                crate::host::set_inspect_max_depth(2);
                out.push_str(&s?);
            }
            'c' => {} // CSS directive: consumes the arg, emits nothing.
            _ => {}
        }
    }
    // Append remaining arguments.
    for a in &args[ai..] {
        out.push(' ');
        out.push_str(&crate::host::console_format_js(a)?);
    }
    Ok(out)
}

// ── promisify / callbackify ──────────────────────────────────────────────────
// These build a REAL JS closure by compiling a factory expression and invoking it
// with the wrapped function — the same re-entrant nested-run path `vm.runInThisContext`
// uses — so the returned value is an ordinary user function (`typeof === "function"`).

/// Compile a single JS expression and run it on the LIVE host, returning its
/// completion value. Delegates to the frontend's ONE runtime-source evaluator
/// (`crate::eval_in_global_scope`), which runs the factory in the program's
/// module scope rather than in the calling function's frame.
fn run_completion(src: &str) -> Result<Value, String> {
    crate::eval_in_global_scope(src)
}

/// A port of `lib/internal/util.js` `promisify`. `custom` is the builtin's own
/// promisified form (node's `kCustomPromisifiedSymbol` on `setTimeout`,
/// `fs.exists`, …) and `argNames` its `kCustomPromisifyArgsSymbol` — the names a
/// multi-value callback resolves into (`{bytesRead, buffer}`); both are supplied
/// from Rust because those symbols are internal. A user function's own
/// `[util.promisify.custom]` is honoured, the result carries the original's
/// prototype and own properties (`name`, `length`), and it is its own custom
/// form, so promisifying it again is the identity.
/// A non-function custom form is returned boxed (`[c]`) for the caller to
/// reject with node's `ERR_INVALID_ARG_TYPE`.
const PROMISIFY_SRC: &str = "(function(original, custom, argNames, sym){\n\
  var c = custom || original[sym];\n\
  if (c) {\n\
    if (typeof c !== 'function') return [c];\n\
    try { Object.defineProperty(c, sym, { value: c, enumerable: false, writable: false, configurable: true }); } catch (_) {}\n\
    return c;\n\
  }\n\
  function fn(...args) {\n\
    var self = this;\n\
    return new Promise(function(resolve, reject){\n\
      args.push(function(err, ...values){\n\
        if (err) return reject(err);\n\
        if (argNames !== undefined && values.length > 1) {\n\
          var obj = {};\n\
          for (var i = 0; i < argNames.length; i++) obj[argNames[i]] = values[i];\n\
          resolve(obj);\n\
        } else {\n\
          resolve(values[0]);\n\
        }\n\
      });\n\
      Reflect.apply(original, self, args);\n\
    });\n\
  }\n\
  Object.setPrototypeOf(fn, Object.getPrototypeOf(original));\n\
  Object.defineProperty(fn, sym, { value: fn, enumerable: false, writable: false, configurable: true });\n\
  return Object.defineProperties(fn, Object.getOwnPropertyDescriptors(original));\n\
})";

/// `fs.exists`'s custom promisified form: it resolves with the boolean its
/// one-argument callback receives, never rejects.
const EXISTS_PROMISIFIED_SRC: &str = "(function exists(path){\n\
  return new Promise(function(resolve){ require('fs').exists(path, resolve); });\n\
})";

/// `child_process`'s `customPromiseExecFunction(orig)`: resolves
/// `{ stdout, stderr }`, or rejects with the error carrying both, and exposes
/// the child as `promise.child`.
const CUSTOM_EXEC_SRC: &str = "(function(orig, name){\n\
  const f = function(...args) {\n\
    const { promise, resolve, reject } = Promise.withResolvers();\n\
    promise.child = orig(...args, (err, stdout, stderr) => {\n\
      if (err !== null) {\n\
        err.stdout = stdout;\n\
        err.stderr = stderr;\n\
        reject(err);\n\
      } else {\n\
        resolve({ stdout, stderr });\n\
      }\n\
    });\n\
    return promise;\n\
  };\n\
  Object.defineProperty(f, 'name', { value: name });\n\
  return f;\n\
})";

/// The function node keeps under `[util.promisify.custom]` on one of its own
/// builtins (`kCustomPromisifiedSymbol`), keyed by the builtin's name here,
/// or `None` for a builtin that has none. It is ONE value per builtin, so a
/// read of `setTimeout[util.promisify.custom]`, `util.promisify(setTimeout)`
/// and `require('timers/promises').setTimeout` are the same function.
pub(crate) fn custom_promisified(builtin: &str) -> Option<Result<Value, String>> {
    let alias = |target: &str| Some(Ok(with_host(|h| h.alloc(JsObj::Builtin(target.into())))));
    match builtin {
        "setTimeout" => return alias("timers/promises.setTimeout"),
        "setImmediate" => return alias("timers/promises.setImmediate"),
        "stream.pipeline" | "Stream.pipeline" => return alias("stream/promises.pipeline"),
        "stream.finished" | "Stream.finished" => return alias("stream/promises.finished"),
        "fs.exists" | "child_process.exec" | "child_process.execFile" => {}
        _ => return None,
    }
    thread_local! {
        static MADE: std::cell::RefCell<std::collections::HashMap<String, Value>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    if let Some(v) = MADE.with(|m| m.borrow().get(builtin).cloned()) {
        return Some(Ok(v));
    }
    let made = (|| {
        if builtin == "fs.exists" {
            return run_completion(EXISTS_PROMISIFIED_SRC);
        }
        let factory = run_completion(CUSTOM_EXEC_SRC)?;
        let (orig, name) = with_host(|h| {
            let short = builtin.trim_start_matches("child_process.");
            (h.alloc(JsObj::Builtin(builtin.into())), h.new_str(short))
        });
        crate::host::invoke(&factory, vec![orig, name], None)
    })();
    if let Ok(v) = &made {
        MADE.with(|m| m.borrow_mut().insert(builtin.to_string(), v.clone()));
    }
    Some(made)
}

const CALLBACKIFY_SRC: &str = "(function(original){\n\
  return function(){\n\
    var self = this;\n\
    var args = Array.prototype.slice.call(arguments);\n\
    var cb = args.pop();\n\
    Promise.resolve(original.apply(self, args)).then(\n\
      function(value){ cb.call(self, null, value); },\n\
      function(err){ cb.call(self, err || new Error('Promise was rejected with a falsy value')); }\n\
    );\n\
  };\n\
})";

/// `util.promisify(fn)` → a function returning a Promise that resolves with the
/// callback's value (rejecting on its error argument).
/// `util.aborted(signal, resource)`.
///
/// Resolves when `signal` aborts — immediately if it already has. The
/// `resource` argument is REQUIRED to be an object (node validates it before
/// looking at the signal); it only ties the promise to an async resource for
/// tracking, which this runtime does not model, so a valid one is then ignored.
fn aborted(signal: &Value, resource: &Value) -> Result<Value, String> {
    if !with_host(|h| {
        matches!(
            h.get(resource),
            Some(crate::host::JsObj::Object(_)) | Some(crate::host::JsObj::Array(_))
        ) || crate::host::is_callable(h, resource)
    }) {
        return Err(crate::host::invalid_arg_type(
            "resource", "argument", "object", resource,
        ));
    }
    let already = crate::builtins::get_property(signal, "aborted").unwrap_or(Value::Undef);
    if with_host(|h| h.truthy(&already)) {
        return crate::builtins::promise_resolve_pub(Value::Undef);
    }
    // The listener is an ordinary `abort` handler, so it goes through the same
    // path a script's own `signal.addEventListener('abort', …)` would — and,
    // as in node (`signal.addEventListener('abort', resolve)`), the promise
    // resolves WITH the abort `Event` the listener receives.
    let (promise, resolve) = crate::builtins::pending_promise_with_resolver();
    crate::host::call_method(
        signal,
        "addEventListener",
        vec![with_host(|h| h.new_str("abort")), resolve],
    )?;
    Ok(promise)
}

fn promisify(args: &[Value]) -> Result<Value, String> {
    let orig = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &orig)) {
        return Err(std::format!(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"original\" argument must be of \
             type function. Received {}",
            super::received_desc(&orig)
        ));
    }
    // The builtins node gives a custom promisified form or callback-argument
    // names. `promisify(setTimeout)` used to wrap it as callback-last, so the
    // `sleep = promisify(setTimeout)` idiom passed the delay as the callback.
    let builtin = with_host(|h| match h.get(&orig) {
        Some(JsObj::Builtin(name)) => Some(name.clone()),
        _ => None,
    });
    let custom = match builtin.as_deref() {
        Some(name) => custom_promisified(name).transpose()?,
        None => None,
    };
    let arg_names: Option<&[&str]> = match builtin.as_deref() {
        Some("fs.read") | Some("fs.readv") => Some(&["bytesRead", "buffer"]),
        Some("fs.write") => Some(&["bytesWritten", "buffer"]),
        Some("fs.writev") => Some(&["bytesWritten", "buffers"]),
        Some("dns.lookup") => Some(&["address", "family"]),
        Some("dns.lookupService") => Some(&["hostname", "service"]),
        _ => None,
    };
    let (custom, names, sym) = with_host(|h| {
        let names = match arg_names {
            Some(list) => {
                let items = list.iter().map(|n| h.new_str(*n)).collect();
                h.new_array(items)
            }
            None => Value::Undef,
        };
        let sym = h.symbol_for("nodejs.util.promisify.custom");
        (custom.unwrap_or(Value::Undef), names, sym)
    });
    let factory = run_completion(PROMISIFY_SRC)?;
    let out = crate::host::invoke(&factory, vec![orig, custom, names, sym], None)?;
    // A non-function `[util.promisify.custom]` comes back boxed in a
    // one-element array, so the rejection can name it as node does.
    if with_host(|h| crate::host::is_callable(h, &out)) {
        return Ok(out);
    }
    let bad = crate::builtins::get_property(&out, "0")?;
    Err(crate::host::invalid_arg_type(
        "util.promisify.custom",
        "property",
        "function",
        &bad,
    ))
}

/// `util.callbackify(fn)` → a function taking a trailing `(err, value)` callback,
/// invoked from the async function's resolved/rejected result.
fn callbackify(args: &[Value]) -> Result<Value, String> {
    let orig = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &orig)) {
        return Err(std::format!(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"original\" argument must be of \
             type function. Received {}",
            super::received_desc(&orig)
        ));
    }
    let factory = run_completion(CALLBACKIFY_SRC)?;
    crate::host::invoke(&factory, vec![orig], None)
}

// ── debuglog ─────────────────────────────────────────────────────────────────

const DEBUGLOG_ENABLED_SRC: &str = "(function(prefix){\n\
  var util = require('util');\n\
  return function(){\n\
    console.error(prefix + ' ' + util.format.apply(null, arguments));\n\
  };\n\
})";

/// `util.debuglog(section)` → a logging function gated by the `NODE_DEBUG` env var.
/// When the section is not enabled, a no-op function is returned (Node's contract).
fn debuglog(args: &[Value]) -> Result<Value, String> {
    let section = super::arg_str(args, 0);
    if debuglog_enabled(&section) {
        let prefix = format!("{} {}:", section.to_uppercase(), std::process::id());
        let factory = run_completion(DEBUGLOG_ENABLED_SRC)?;
        let pfx = with_host(|h| h.new_str(prefix));
        crate::host::invoke(&factory, vec![pfx], None)
    } else {
        run_completion("(function(){})")
    }
}

/// Whether `NODE_DEBUG` enables `section` (comma/space-separated, case-insensitive,
/// `*` wildcards allowed — matching Node's env parsing).
fn debuglog_enabled(section: &str) -> bool {
    let Ok(env) = std::env::var("NODE_DEBUG") else {
        return false;
    };
    let sec = section.to_uppercase();
    env.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .any(|pat| {
            let pat = pat.to_uppercase();
            if pat.contains('*') {
                wildcard_match(&pat, &sec)
            } else {
                pat == sec
            }
        })
}

/// Minimal glob match (`*` = any run) for `NODE_DEBUG` section patterns.
fn wildcard_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !s[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if i == parts.len() - 1 {
            return s[pos..].ends_with(part);
        } else if let Some(idx) = s[pos..].find(part) {
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    true
}

// ── stripVTControlCharacters ─────────────────────────────────────────────────

/// Remove ANSI/VT escape sequences: two-char escapes, CSI (`ESC [ … final`),
/// and OSC (`ESC ] … BEL|ST`), including the C1 CSI introducer ``.
pub(crate) fn strip_vt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{9b}' {
            // C1 CSI: consume params/intermediates until a final byte.
            while let Some(&n) = chars.peek() {
                chars.next();
                if ('\u{40}'..='\u{7e}').contains(&n) {
                    break;
                }
            }
            continue;
        }
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(&n) = chars.peek() {
                    if n == '\u{7}' {
                        chars.next();
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    chars.next();
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

// ── styleText ────────────────────────────────────────────────────────────────

/// The format name(s) passed to `styleText` (a string or an array of strings).
fn style_names(v: &Value) -> Vec<String> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Array(items)) => items.iter().map(|x| h.str_of(x)).collect(),
        _ => vec![h.str_of(v)],
    })
}

/// `(open, close)` SGR parameter numbers for a `util.inspect.colors` name.
/// `util.inspect.colors`: every SGR style name Node accepts, as
/// `(name, open, close)`, in `Object.keys(util.inspect.colors)` order.
///
/// One table rather than a `match`, because `styleText`'s rejection message
/// ENUMERATES these names and the enumeration has to come from the same source
/// the lookup does. The old code had a `match` for the lookup and a
/// hand-written sentence for the message, so the sentence could not go stale
/// visibly — and it had: it said "must be a valid util.inspect.colors key",
/// a wording no Node emits.
pub(crate) const STYLES: &[(&str, u16, u16)] = &[
    ("reset", 0, 0),
    ("bold", 1, 22),
    ("dim", 2, 22),
    ("italic", 3, 23),
    ("underline", 4, 24),
    ("blink", 5, 25),
    ("inverse", 7, 27),
    ("hidden", 8, 28),
    ("strikethrough", 9, 29),
    ("doubleunderline", 21, 24),
    ("black", 30, 39),
    ("red", 31, 39),
    ("green", 32, 39),
    ("yellow", 33, 39),
    ("blue", 34, 39),
    ("magenta", 35, 39),
    ("cyan", 36, 39),
    ("white", 37, 39),
    ("bgBlack", 40, 49),
    ("bgRed", 41, 49),
    ("bgGreen", 42, 49),
    ("bgYellow", 43, 49),
    ("bgBlue", 44, 49),
    ("bgMagenta", 45, 49),
    ("bgCyan", 46, 49),
    ("bgWhite", 47, 49),
    ("framed", 51, 54),
    ("overlined", 53, 55),
    ("gray", 90, 39),
    ("redBright", 91, 39),
    ("greenBright", 92, 39),
    ("yellowBright", 93, 39),
    ("blueBright", 94, 39),
    ("magentaBright", 95, 39),
    ("cyanBright", 96, 39),
    ("whiteBright", 97, 39),
    ("bgGray", 100, 49),
    ("bgRedBright", 101, 49),
    ("bgGreenBright", 102, 49),
    ("bgYellowBright", 103, 49),
    ("bgBlueBright", 104, 49),
    ("bgMagentaBright", 105, 49),
    ("bgCyanBright", 106, 49),
    ("bgWhiteBright", 107, 49),
];

/// The aliases Node accepts but does not list in `inspect.colors`' own keys. It
/// still names them in the rejection message, after the listed keys.
pub(crate) const STYLE_ALIASES: &[(&str, u16, u16)] = &[
    ("grey", 90, 39),
    ("blackBright", 90, 39),
    ("bgGrey", 100, 49),
    ("bgBlackBright", 100, 49),
    ("faint", 2, 22),
    ("crossedout", 9, 29),
    ("strikeThrough", 9, 29),
    ("crossedOut", 9, 29),
    ("conceal", 8, 28),
    ("swapColors", 7, 27),
    ("swapcolors", 7, 27),
    ("doubleUnderline", 21, 24),
];

/// The `(open, close)` SGR pair for a style name, or `None` if unknown.
fn style_codes(name: &str) -> Option<(u16, u16)> {
    STYLES
        .iter()
        .chain(STYLE_ALIASES.iter())
        .find(|(n, _, _)| *n == name)
        .map(|(_, o, c)| (*o, *c))
}

/// `util.styleText(format, text[, { validateStream = true, stream = process.stdout }])`,
/// a port of `lib/util.js`. Each format's opening code goes on the left in order
/// and its closing code on the right in reverse, so the first is outermost; a
/// format's closing code inside the text is replaced by its opening code (for
/// the codes `bold`/`dim` share, the close is kept and the open re-applied), so
/// a nested `styleText` does not end the outer style early. `#RGB`/`#RRGGBB` is
/// a 24-bit color. With `validateStream` the stream must be a stream, and when
/// it would not colorize — not a TTY, or `FORCE_COLOR` disabling it — the text
/// comes back unstyled: piped output used to carry raw escape codes.
fn style_text(args: &[Value]) -> Result<Value, String> {
    let fmt = args.first().cloned().unwrap_or(Value::Undef);
    let text_v = args.get(1).cloned().unwrap_or(Value::Undef);
    let Some(text) = with_host(|h| h.as_str(&text_v)) else {
        return Err(crate::host::invalid_arg_type(
            "text", "argument", "string", &text_v,
        ));
    };
    let options = args.get(2).cloned().unwrap_or(Value::Undef);
    let has_options = !matches!(options, Value::Undef);
    if has_options && !is_plain_object_arg(&options) {
        return Err(crate::host::invalid_arg_type(
            "options", "argument", "object", &options,
        ));
    }
    let opt = |k: &str| -> Result<Value, String> {
        if has_options {
            crate::builtins::get_property(&options, k)
        } else {
            Ok(Value::Undef)
        }
    };
    let validate_stream = match opt("validateStream")? {
        Value::Undef => true,
        Value::Bool(b) => b,
        v => {
            return Err(crate::host::invalid_arg_type(
                "options.validateStream",
                "property",
                "boolean",
                &v,
            ))
        }
    };
    let mut skip_colorize = false;
    if validate_stream {
        let stream = match opt("stream")? {
            Value::Undef => crate::builtins::namespace_property("process", "stdout"),
            s => s,
        };
        if !is_stream_like(&stream)? {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                &format!(
                    "The \"stream\" argument must be an instance of ReadableStream, WritableStream, or Stream. Received {}",
                    super::received_desc(&stream)
                ),
            ));
        }
        skip_colorize = !should_colorize(&stream)?;
    }
    let (mut open_codes, mut close_codes, mut processed) =
        (String::new(), String::new(), text.clone());
    for name in style_names(&fmt) {
        if name == "none" {
            continue;
        }
        let (open, close, keep_close) = if let Some(hex) = name.strip_prefix('#') {
            let Some((r, g, b)) = hex_rgb(hex) else {
                return Err(crate::host::coded_error(
                    "TypeError",
                    "ERR_INVALID_ARG_VALUE",
                    &format!(
                        "The argument 'format' must be a valid hex color (#RGB or #RRGGBB). Received '{name}'"
                    ),
                ));
            };
            if skip_colorize {
                continue;
            }
            (
                format!("\u{1b}[38;2;{r};{g};{b}m"),
                "\u{1b}[39m".to_string(),
                false,
            )
        } else {
            let (open, close) = style_codes(&name).ok_or_else(|| {
                // Node spells out every accepted name. Generated from the same
                // table the lookup reads, so a style can never be
                // accepted-but-unlisted.
                let listed: Vec<String> = STYLES
                    .iter()
                    .chain(STYLE_ALIASES.iter())
                    .map(|(n, _, _)| format!("'{n}'"))
                    .collect();
                crate::host::coded_error(
                    "TypeError",
                    "ERR_INVALID_ARG_VALUE",
                    &format!(
                        "The argument 'format' must be one of: {}. Received '{name}'",
                        listed.join(", ")
                    ),
                )
            })?;
            (
                format!("\u{1b}[{open}m"),
                format!("\u{1b}[{close}m"),
                close == 22,
            )
        };
        open_codes.push_str(&open);
        close_codes.insert_str(0, &close);
        let replacement = if keep_close {
            format!("{close}{open}")
        } else {
            open
        };
        processed = processed.replace(&close, &replacement);
    }
    let out = if skip_colorize {
        text
    } else {
        format!("{open_codes}{processed}{close_codes}")
    };
    Ok(with_host(|h| h.new_str(out)))
}

/// `#RGB` / `#RRGGBB` (the part after `#`) as its three channels.
fn hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let d = |i: usize| channel(&hex[i..i + 1].repeat(2));
            Some((d(0)?, d(1)?, d(2)?))
        }
        6 => Some((
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
        )),
        _ => None,
    }
}

/// `validateObject(options, 'options')`: an object, not `null`, not an array.
fn is_plain_object_arg(v: &Value) -> bool {
    with_host(|h| {
        h.type_of(v) == "object" && !h.is_null(v) && !matches!(h.get(v), Some(JsObj::Array(_)))
    })
}

/// node's `isReadableStream || isWritableStream || isNodeStream`.
fn is_stream_like(v: &Value) -> Result<bool, String> {
    if super::stream_web::is_readable_stream(v)? || super::stream_web::is_writable_stream(v)? {
        return Ok(true);
    }
    if !with_host(|h| matches!(h.type_of(v), "object" | "function") && !h.is_null(v)) {
        return Ok(false);
    }
    let get = |k: &str| crate::builtins::get_property(v, k);
    let truthy = |x: &Value| with_host(|h| h.truthy(x));
    let callable = |x: &Value| with_host(|h| crate::host::is_callable(h, x));
    if truthy(&get("_readableState")?) || truthy(&get("_writableState")?) {
        return Ok(true);
    }
    let on = callable(&get("on")?);
    Ok(on && (callable(&get("write")?) || callable(&get("pipe")?)))
}

/// `utilColors.shouldColorize(stream)`: with `FORCE_COLOR` set, whether its
/// color depth exceeds 2 (`''`, `1`, `true`, `2`, `3` do); otherwise whether the
/// stream is a TTY whose `getColorDepth()` (when it has one) exceeds 2.
fn should_colorize(stream: &Value) -> Result<bool, String> {
    if let Ok(fc) = std::env::var("FORCE_COLOR") {
        return Ok(matches!(fc.as_str(), "" | "1" | "true" | "2" | "3"));
    }
    let is_tty = crate::builtins::get_property(stream, "isTTY")?;
    if !with_host(|h| h.truthy(&is_tty)) {
        return Ok(false);
    }
    let depth = crate::builtins::get_property(stream, "getColorDepth")?;
    if !with_host(|h| crate::host::is_callable(h, &depth)) {
        return Ok(true);
    }
    let d = crate::host::invoke(&depth, Vec::new(), Some(stream.clone()))?;
    Ok(with_host(|h| h.to_number(&d)) > 2.0)
}

// ── getSystemError{Name,Message,Map} ─────────────────────────────────────────
// Node's system error numbers are negative libuv errnos. We resolve them through
// the platform's own `libc` constants so macOS/Linux each report their native
// numbering, and pair each with libuv's canonical (lowercase) message string.

/// `(name, platform errno, libuv message)` for the common POSIX errors. Only
/// constants present on every supported Unix target are listed (cross-platform).
const ERRNO_TABLE: &[(&str, i32, &str)] = &[
    ("E2BIG", libc::E2BIG, "argument list too long"),
    ("EACCES", libc::EACCES, "permission denied"),
    ("EADDRINUSE", libc::EADDRINUSE, "address already in use"),
    (
        "EADDRNOTAVAIL",
        libc::EADDRNOTAVAIL,
        "address not available",
    ),
    (
        "EAFNOSUPPORT",
        libc::EAFNOSUPPORT,
        "address family not supported",
    ),
    ("EAGAIN", libc::EAGAIN, "resource temporarily unavailable"),
    ("EALREADY", libc::EALREADY, "connection already in progress"),
    ("EBADF", libc::EBADF, "bad file descriptor"),
    ("EBUSY", libc::EBUSY, "resource busy or locked"),
    ("ECANCELED", libc::ECANCELED, "operation canceled"),
    (
        "ECONNABORTED",
        libc::ECONNABORTED,
        "software caused connection abort",
    ),
    ("ECONNREFUSED", libc::ECONNREFUSED, "connection refused"),
    ("ECONNRESET", libc::ECONNRESET, "connection reset by peer"),
    (
        "EDESTADDRREQ",
        libc::EDESTADDRREQ,
        "destination address required",
    ),
    ("EEXIST", libc::EEXIST, "file already exists"),
    (
        "EFAULT",
        libc::EFAULT,
        "bad address in system call argument",
    ),
    ("EFBIG", libc::EFBIG, "file too large"),
    ("EHOSTDOWN", libc::EHOSTDOWN, "host is down"),
    ("EHOSTUNREACH", libc::EHOSTUNREACH, "host is unreachable"),
    ("EINTR", libc::EINTR, "interrupted system call"),
    ("EINVAL", libc::EINVAL, "invalid argument"),
    ("EIO", libc::EIO, "i/o error"),
    ("EISCONN", libc::EISCONN, "socket is already connected"),
    ("EISDIR", libc::EISDIR, "illegal operation on a directory"),
    ("ELOOP", libc::ELOOP, "too many symbolic links encountered"),
    ("EMFILE", libc::EMFILE, "too many open files"),
    ("EMLINK", libc::EMLINK, "too many links"),
    ("EMSGSIZE", libc::EMSGSIZE, "message too long"),
    ("ENAMETOOLONG", libc::ENAMETOOLONG, "name too long"),
    ("ENETDOWN", libc::ENETDOWN, "network is down"),
    ("ENETUNREACH", libc::ENETUNREACH, "network is unreachable"),
    ("ENFILE", libc::ENFILE, "file table overflow"),
    ("ENOBUFS", libc::ENOBUFS, "no buffer space available"),
    ("ENODEV", libc::ENODEV, "no such device"),
    ("ENOENT", libc::ENOENT, "no such file or directory"),
    ("ENOMEM", libc::ENOMEM, "not enough memory"),
    ("ENOPROTOOPT", libc::ENOPROTOOPT, "protocol not available"),
    ("ENOSPC", libc::ENOSPC, "no space left on device"),
    ("ENOSYS", libc::ENOSYS, "function not implemented"),
    ("ENOTCONN", libc::ENOTCONN, "socket is not connected"),
    ("ENOTDIR", libc::ENOTDIR, "not a directory"),
    ("ENOTEMPTY", libc::ENOTEMPTY, "directory not empty"),
    ("ENOTSOCK", libc::ENOTSOCK, "socket operation on non-socket"),
    ("ENXIO", libc::ENXIO, "no such device or address"),
    (
        "EOPNOTSUPP",
        libc::EOPNOTSUPP,
        "operation not supported on socket",
    ),
    (
        "EOVERFLOW",
        libc::EOVERFLOW,
        "value too large for defined data type",
    ),
    ("EPERM", libc::EPERM, "operation not permitted"),
    ("EPIPE", libc::EPIPE, "broken pipe"),
    ("EPROTO", libc::EPROTO, "protocol error"),
    (
        "EPROTONOSUPPORT",
        libc::EPROTONOSUPPORT,
        "protocol not supported",
    ),
    (
        "EPROTOTYPE",
        libc::EPROTOTYPE,
        "protocol wrong type for socket",
    ),
    ("ERANGE", libc::ERANGE, "result too large"),
    ("EROFS", libc::EROFS, "read-only file system"),
    (
        "ESHUTDOWN",
        libc::ESHUTDOWN,
        "cannot send after transport endpoint shutdown",
    ),
    ("ESPIPE", libc::ESPIPE, "invalid seek"),
    ("ESRCH", libc::ESRCH, "no such process"),
    ("ETIMEDOUT", libc::ETIMEDOUT, "connection timed out"),
    ("ETXTBSY", libc::ETXTBSY, "text file is busy"),
    ("EXDEV", libc::EXDEV, "cross-device link not permitted"),
];

/// Normalize a `getSystemError*` argument (a negative libuv errno) to a positive
/// platform errno for table lookup.
fn errno_of(err: f64) -> i32 {
    if err < 0.0 {
        (-err) as i32
    } else {
        err as i32
    }
}

pub(crate) fn errno_name(e: i32) -> Option<&'static str> {
    ERRNO_TABLE
        .iter()
        .find(|(_, code, _)| *code == e)
        .map(|(n, _, _)| *n)
}

fn errno_message(e: i32) -> Option<&'static str> {
    ERRNO_TABLE
        .iter()
        .find(|(_, code, _)| *code == e)
        .map(|(_, _, m)| *m)
}

/// The platform errno number and libuv message for an error CODE (`EBADF` →
/// `(9, "bad file descriptor")`), from the same table `util.getSystemErrorMap`
/// reports.
pub(crate) fn errno_by_name(name: &str) -> Option<(i32, &'static str)> {
    ERRNO_TABLE
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, code, m)| (*code, *m))
}

/// `util.getSystemErrorMap()` → a `Map` of negative errno → `[name, message]`.
fn system_error_map() -> Value {
    with_host(|h| {
        let mut entries = indexmap::IndexMap::new();
        for (name, code, msg) in ERRNO_TABLE {
            let key_val = Value::Float(-(*code as f64));
            let name_v = h.new_str(*name);
            let msg_v = h.new_str(*msg);
            let pair = h.new_array(vec![name_v, msg_v]);
            let key = crate::host::map_key(h, &key_val);
            entries.insert(key, (key_val, pair));
        }
        h.alloc(JsObj::Map {
            entries,
            weak: false,
        })
    })
}

// ── parseArgs ────────────────────────────────────────────────────────────────

/// A port of node's `lib/internal/util/parse_args/parse_args.js` and
/// `utils.js` (v26.12.0), with the primordials spelled as ordinary method
/// calls. It runs as JavaScript because it IS JavaScript in node: the token
/// stream (`tokens: true`), the order every validation and usage check runs
/// in, and the `__proto__`/null-prototype handling all fall out of the same
/// code. The errors node's `internal/errors` would construct are raised by
/// `fail` (`util.@@nodeError`, below), so they carry node's codes and
/// wording.
const PARSE_ARGS_SRC: &str = r#"(function (fail, evalMode) {
'use strict';
const ObjectHasOwn = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
const uncurry = (f) => (self, ...args) => Reflect.apply(f, self, args);
const StringPrototypeCharAt = uncurry(String.prototype.charAt);
const StringPrototypeIncludes = uncurry(String.prototype.includes);
const StringPrototypeIndexOf = uncurry(String.prototype.indexOf);
const StringPrototypeSlice = uncurry(String.prototype.slice);
const StringPrototypeStartsWith = uncurry(String.prototype.startsWith);
function ERR_INVALID_ARG_TYPE(name, expected, value) { fail('type', name, expected, value); }
function ERR_INVALID_ARG_VALUE(name, value, reason) { fail('value', name, value, reason); }
function ERR_PARSE_ARGS_INVALID_OPTION_VALUE(message) { fail('code', 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE', message); }
function ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL(value) {
  fail('code', 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL', `Unexpected argument '${value}'. This command does not take positional arguments`);
}
function ERR_PARSE_ARGS_UNKNOWN_OPTION(option, allowPositionals) {
  const suggestDashDash = allowPositionals ? '. To specify a positional ' +
    "argument starting with a '-', place it at the end of the command after " +
    `'--', as in '-- ${JSON.stringify(option)}` : '';
  fail('code', 'ERR_PARSE_ARGS_UNKNOWN_OPTION', `Unknown option '${option}'${suggestDashDash}`);
}

// internal/validators
function validateObject(value, name) {
  if (value === null || Array.isArray(value)) throw new ERR_INVALID_ARG_TYPE(name, 'Object', value);
  if (typeof value !== 'object') throw new ERR_INVALID_ARG_TYPE(name, 'Object', value);
}
function validateArray(value, name, minLength = 0) {
  if (!Array.isArray(value)) throw new ERR_INVALID_ARG_TYPE(name, 'Array', value);
  if (value.length < minLength) throw new ERR_INVALID_ARG_VALUE(name, value, `must be longer than ${minLength}`);
}
function validateString(value, name) {
  if (typeof value !== 'string') throw new ERR_INVALID_ARG_TYPE(name, 'string', value);
}
function validateBoolean(value, name) {
  if (typeof value !== 'boolean') throw new ERR_INVALID_ARG_TYPE(name, 'boolean', value);
}
function validateStringArray(value, name) {
  validateArray(value, name);
  for (let i = 0; i < value.length; ++i) validateString(value[i], `${name}[${i}]`);
}
function validateBooleanArray(value, name) {
  validateArray(value, name);
  for (let i = 0; i < value.length; ++i) validateBoolean(value[i], `${name}[${i}]`);
}
function validateUnion(value, name, union) {
  if (!union.includes(value)) throw new ERR_INVALID_ARG_TYPE(name, `('${union.join('|')}')`, value);
}

// internal/util/parse_args/utils
function objectGetOwn(obj, prop) {
  if (ObjectHasOwn(obj, prop))
    return obj[prop];
}
function optionsGetOwn(options, longOption, prop) {
  if (ObjectHasOwn(options, longOption))
    return objectGetOwn(options[longOption], prop);
}
function isOptionValue(value) {
  if (value == null) return false;
  return true; // greedy!
}
function isOptionLikeValue(value) {
  if (value == null) return false;
  return value.length > 1 && StringPrototypeCharAt(value, 0) === '-';
}
function isLoneShortOption(arg) {
  return arg.length === 2 && StringPrototypeCharAt(arg, 0) === '-' && StringPrototypeCharAt(arg, 1) !== '-';
}
function isLoneLongOption(arg) {
  return arg.length > 2 && StringPrototypeStartsWith(arg, '--') && !StringPrototypeIncludes(arg, '=', 3);
}
function isLongOptionAndValue(arg) {
  return arg.length > 2 && StringPrototypeStartsWith(arg, '--') && StringPrototypeIncludes(arg, '=', 3);
}
function isShortOptionGroup(arg, options) {
  if (arg.length <= 2) return false;
  if (StringPrototypeCharAt(arg, 0) !== '-') return false;
  if (StringPrototypeCharAt(arg, 1) === '-') return false;
  const firstShort = StringPrototypeCharAt(arg, 1);
  const longOption = findLongOptionForShort(firstShort, options);
  return optionsGetOwn(options, longOption, 'type') !== 'string';
}
function isShortOptionAndValue(arg, options) {
  validateObject(options, 'options');
  if (arg.length <= 2) return false;
  if (StringPrototypeCharAt(arg, 0) !== '-') return false;
  if (StringPrototypeCharAt(arg, 1) === '-') return false;
  const shortOption = StringPrototypeCharAt(arg, 1);
  const longOption = findLongOptionForShort(shortOption, options);
  return optionsGetOwn(options, longOption, 'type') === 'string';
}
function findLongOptionForShort(shortOption, options) {
  validateObject(options, 'options');
  const longOptionEntry = Object.entries(options).find(
    ({ 1: optionConfig }) => objectGetOwn(optionConfig, 'short') === shortOption,
  );
  return longOptionEntry?.[0] ?? shortOption;
}
function useDefaultValueOption(longOption, optionConfig, values) {
  return objectGetOwn(optionConfig, 'default') !== undefined &&
    values[longOption] === undefined;
}

// internal/util/parse_args/parse_args
function getMainArgs() {
  if (evalMode) return process.argv.slice(1);
  return process.argv.slice(2);
}
function checkOptionLikeValue(token) {
  if (!token.inlineValue && isOptionLikeValue(token.value)) {
    const example = StringPrototypeStartsWith(token.rawName, '--') ?
      `'${token.rawName}=-XYZ'` :
      `'--${token.name}=-XYZ' or '${token.rawName}-XYZ'`;
    const errorMessage = `Option '${token.rawName}' argument is ambiguous.
Did you forget to specify the option argument for '${token.rawName}'?
To specify an option argument starting with a dash use ${example}.`;
    throw new ERR_PARSE_ARGS_INVALID_OPTION_VALUE(errorMessage);
  }
}
function checkOptionUsage(config, token) {
  let tokenName = token.name;
  if (!ObjectHasOwn(config.options, tokenName)) {
    if (config.allowNegative && StringPrototypeStartsWith(tokenName, 'no-')) {
      tokenName = StringPrototypeSlice(tokenName, 3);
      if (!ObjectHasOwn(config.options, tokenName) || optionsGetOwn(config.options, tokenName, 'type') !== 'boolean') {
        throw new ERR_PARSE_ARGS_UNKNOWN_OPTION(token.rawName, config.allowPositionals);
      }
    } else {
      throw new ERR_PARSE_ARGS_UNKNOWN_OPTION(token.rawName, config.allowPositionals);
    }
  }
  const short = optionsGetOwn(config.options, tokenName, 'short');
  const shortAndLong = `${short ? `-${short}, ` : ''}--${tokenName}`;
  const type = optionsGetOwn(config.options, tokenName, 'type');
  if (type === 'string' && typeof token.value !== 'string') {
    throw new ERR_PARSE_ARGS_INVALID_OPTION_VALUE(`Option '${shortAndLong} <value>' argument missing`);
  }
  if (type === 'boolean' && token.value != null) {
    throw new ERR_PARSE_ARGS_INVALID_OPTION_VALUE(`Option '${shortAndLong}' does not take an argument`);
  }
}
function storeOption(token, options, values, allowNegative) {
  let longOption = token.name;
  let optionValue = token.value;
  if (longOption === '__proto__') {
    return;
  }
  if (allowNegative && StringPrototypeStartsWith(longOption, 'no-') && optionValue === undefined) {
    longOption = StringPrototypeSlice(longOption, 3);
    token.name = longOption;
    optionValue = false;
  }
  const newValue = optionValue ?? true;
  if (optionsGetOwn(options, longOption, 'multiple')) {
    if (values[longOption]) {
      values[longOption].push(newValue);
    } else {
      values[longOption] = [newValue];
    }
  } else {
    values[longOption] = newValue;
  }
}
function storeDefaultOption(longOption, optionValue, values) {
  if (longOption === '__proto__') {
    return;
  }
  values[longOption] = optionValue;
}
function argsToTokens(args, options) {
  const tokens = [];
  let index = -1;
  let groupCount = 0;
  const remainingArgs = args.slice();
  while (remainingArgs.length > 0) {
    const arg = remainingArgs.shift();
    const nextArg = remainingArgs[0];
    if (groupCount > 0)
      groupCount--;
    else
      index++;
    if (arg === '--') {
      tokens.push({ kind: 'option-terminator', index });
      tokens.push(...remainingArgs.map((arg) => {
        return { kind: 'positional', index: ++index, value: arg };
      }));
      break;
    }
    if (isLoneShortOption(arg)) {
      const shortOption = StringPrototypeCharAt(arg, 1);
      const longOption = findLongOptionForShort(shortOption, options);
      let value;
      let inlineValue;
      if (optionsGetOwn(options, longOption, 'type') === 'string' &&
          isOptionValue(nextArg)) {
        value = remainingArgs.shift();
        inlineValue = false;
      }
      tokens.push({ kind: 'option', name: longOption, rawName: arg, index, value, inlineValue });
      if (value != null) ++index;
      continue;
    }
    if (isShortOptionGroup(arg, options)) {
      const expanded = [];
      for (let index = 1; index < arg.length; index++) {
        const shortOption = StringPrototypeCharAt(arg, index);
        const longOption = findLongOptionForShort(shortOption, options);
        if (optionsGetOwn(options, longOption, 'type') !== 'string' ||
          index === arg.length - 1) {
          expanded.push(`-${shortOption}`);
        } else {
          expanded.push(`-${StringPrototypeSlice(arg, index)}`);
          break;
        }
      }
      remainingArgs.unshift(...expanded);
      groupCount = expanded.length;
      continue;
    }
    if (isShortOptionAndValue(arg, options)) {
      const shortOption = StringPrototypeCharAt(arg, 1);
      const longOption = findLongOptionForShort(shortOption, options);
      const value = StringPrototypeSlice(arg, 2);
      tokens.push({ kind: 'option', name: longOption, rawName: `-${shortOption}`, index, value, inlineValue: true });
      continue;
    }
    if (isLoneLongOption(arg)) {
      const longOption = StringPrototypeSlice(arg, 2);
      let value;
      let inlineValue;
      if (optionsGetOwn(options, longOption, 'type') === 'string' &&
          isOptionValue(nextArg)) {
        value = remainingArgs.shift();
        inlineValue = false;
      }
      tokens.push({ kind: 'option', name: longOption, rawName: arg, index, value, inlineValue });
      if (value != null) ++index;
      continue;
    }
    if (isLongOptionAndValue(arg)) {
      const equalIndex = StringPrototypeIndexOf(arg, '=');
      const longOption = StringPrototypeSlice(arg, 2, equalIndex);
      const value = StringPrototypeSlice(arg, equalIndex + 1);
      tokens.push({ kind: 'option', name: longOption, rawName: `--${longOption}`, index, value, inlineValue: true });
      continue;
    }
    tokens.push({ kind: 'positional', index, value: arg });
  }
  return tokens;
}
const parseArgs = (config = {}) => {
  const args = objectGetOwn(config, 'args') ?? getMainArgs();
  const strict = objectGetOwn(config, 'strict') ?? true;
  const allowPositionals = objectGetOwn(config, 'allowPositionals') ?? !strict;
  const returnTokens = objectGetOwn(config, 'tokens') ?? false;
  const allowNegative = objectGetOwn(config, 'allowNegative') ?? false;
  const options = objectGetOwn(config, 'options') ?? { __proto__: null };
  const parseConfig = { args, strict, options, allowPositionals, allowNegative };
  validateArray(args, 'args');
  validateBoolean(strict, 'strict');
  validateBoolean(allowPositionals, 'allowPositionals');
  validateBoolean(returnTokens, 'tokens');
  validateBoolean(allowNegative, 'allowNegative');
  validateObject(options, 'options');
  Object.entries(options).forEach(
    ({ 0: longOption, 1: optionConfig }) => {
      validateObject(optionConfig, `options.${longOption}`);
      const optionType = objectGetOwn(optionConfig, 'type');
      validateUnion(optionType, `options.${longOption}.type`, ['string', 'boolean']);
      if (ObjectHasOwn(optionConfig, 'short')) {
        const shortOption = optionConfig.short;
        validateString(shortOption, `options.${longOption}.short`);
        if (shortOption.length !== 1) {
          throw new ERR_INVALID_ARG_VALUE(
            `options.${longOption}.short`,
            shortOption,
            'must be a single character',
          );
        }
      }
      const multipleOption = objectGetOwn(optionConfig, 'multiple');
      if (ObjectHasOwn(optionConfig, 'multiple')) {
        validateBoolean(multipleOption, `options.${longOption}.multiple`);
      }
      const defaultValue = objectGetOwn(optionConfig, 'default');
      if (defaultValue !== undefined) {
        let validator;
        switch (optionType) {
          case 'string':
            validator = multipleOption ? validateStringArray : validateString;
            break;
          case 'boolean':
            validator = multipleOption ? validateBooleanArray : validateBoolean;
            break;
        }
        validator(defaultValue, `options.${longOption}.default`);
      }
    },
  );
  const tokens = argsToTokens(args, options);
  const result = {
    values: { __proto__: null },
    positionals: [],
  };
  if (returnTokens) {
    result.tokens = tokens;
  }
  tokens.forEach((token) => {
    if (token.kind === 'option') {
      if (strict) {
        checkOptionUsage(parseConfig, token);
        checkOptionLikeValue(token);
      }
      storeOption(token, options, result.values, parseConfig.allowNegative);
    } else if (token.kind === 'positional') {
      if (!allowPositionals) {
        throw new ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL(token.value);
      }
      result.positionals.push(token.value);
    }
  });
  Object.entries(options).forEach(({ 0: longOption, 1: optionConfig }) => {
    const mustSetDefault = useDefaultValueOption(longOption, optionConfig, result.values);
    if (mustSetDefault) {
      storeDefaultOption(longOption, objectGetOwn(optionConfig, 'default'), result.values);
    }
  });
  return result;
};
return parseArgs;
})"#;

/// `util.parseArgs(config)`: the cached port above, called with `config`.
fn parse_args(args: &[Value]) -> Result<Value, String> {
    thread_local! {
        static PARSER: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }
    let parser = match PARSER.with(|p| p.borrow().clone()) {
        Some(p) => p,
        None => {
            let factory = run_completion(PARSE_ARGS_SRC)?;
            let fail = with_host(|h| h.alloc(JsObj::Builtin("util.@@nodeError".into())));
            let eval_mode = Value::Bool(super::process::eval_entry());
            let made = crate::host::invoke(&factory, vec![fail, eval_mode], None)?;
            PARSER.with(|p| *p.borrow_mut() = Some(made.clone()));
            made
        }
    };
    let config = args.first().cloned().unwrap_or(Value::Undef);
    crate::host::invoke(&parser, vec![config], None)
}

/// `util.@@nodeError(kind, …)`: the errors node's `internal/errors` builds,
/// for the JavaScript ports (`parseArgs`, `FileHandle`) to throw. `kind` is
/// `type` (`ERR_INVALID_ARG_TYPE`, given the validator's expected-type
/// spelling), `value` (`ERR_INVALID_ARG_VALUE`) or `code` (any code, as a
/// `TypeError`, with its finished message) or `range` (`ERR_OUT_OF_RANGE`). Always
/// throws. A `:<Class>` suffix on `value`, `code` or `error` names the error
/// class, as node's `ERR_INVALID_ARG_VALUE.RangeError` does
/// (`value:RangeError`, `code:RangeError`).
fn node_error(args: &[Value]) -> Result<Value, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Undef);
    let kind_arg = super::arg_str(args, 0);
    let (kind, class) = kind_arg.split_once(':').unwrap_or((kind_arg.as_str(), ""));
    let class_or = |default: &str| {
        if class.is_empty() {
            default.to_string()
        } else {
            class.to_string()
        }
    };
    let name = super::arg_str(args, 1);
    // `ERR_INVALID_ARG_TYPE` / `ERR_INVALID_ARG_VALUE` call a dotted name a
    // property and anything else an argument.
    let noun = if name.contains('.') {
        "property"
    } else {
        "argument"
    };
    Err(match kind {
        "type" => {
            let spec = super::arg_str(args, 2);
            let expected = match spec.as_str() {
                "Array" => "an instance of Array".to_string(),
                "Object" => "of type object".to_string(),
                // A finished phrase (`('a|b')`, `an instance of Buffer, …`).
                s if s.starts_with('(') || s.contains(' ') => s.to_string(),
                s => format!("of type {s}"),
            };
            crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                &format!(
                    "The \"{name}\" {noun} must be {expected}. Received {}",
                    super::received_desc(&arg(3))
                ),
            )
        }
        "value" => {
            let mut shown = crate::host::inspect_js(&arg(2))?;
            if shown.chars().count() > 128 {
                shown = format!("{}...", shown.chars().take(128).collect::<String>());
            }
            let reason = match arg(3) {
                Value::Undef => "is invalid".to_string(),
                v => with_host(|h| h.str_of(&v)),
            };
            crate::host::coded_error(
                &class_or("TypeError"),
                "ERR_INVALID_ARG_VALUE",
                &format!("The {noun} '{name}' {reason}. Received {shown}"),
            )
        }
        // `ERR_OUT_OF_RANGE(name, must, value)` for a numeric value.
        "range" => {
            let n = with_host(|h| h.to_number(&arg(3)));
            super::out_of_range(&name, &super::arg_str(args, 2), n)
        }
        // A code node raises as a plain `Error` (`ERR_INVALID_STATE`).
        "error" => crate::host::coded_error(&class_or("Error"), &name, &super::arg_str(args, 2)),
        _ => crate::host::coded_error(&class_or("TypeError"), &name, &super::arg_str(args, 2)),
    })
}

// ── parseEnv ─────────────────────────────────────────────────────────────────
// Faithful port of Node's C++ `Dotenv::ParseContent` (src/node_dotenv.cc): CRLF
// normalization, `#`/blank-line skipping, `export ` prefix stripping, single /
// double / backtick quotes (`\n` expanded only inside double quotes), unquoted
// inline `#` comments, unterminated quotes, and last-wins on duplicate keys.
// Node emits the keys sorted, so we `sort_keys` before materializing.

/// Trim ASCII whitespace (space, tab, newline) from both ends, matching Node's
/// `trim_spaces` (which trims only `" \t\n"`).
fn env_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n')
}

/// `util.parseEnv(content)` → an object of parsed `KEY=VALUE` assignments.
fn parse_env(input: &str) -> Value {
    let lines = input.replace('\r', "");
    let mut pairs: IndexMap<String, String> = IndexMap::new();
    let mut content: &str = env_trim(&lines);

    while !content.is_empty() {
        let first = content.as_bytes()[0];
        // Skip blank lines and full-line comments.
        if first == b'\n' || first == b'#' {
            match content.find('\n') {
                Some(nl) => content = &content[nl + 1..],
                None => content = "",
            }
            continue;
        }
        // Next `=` or newline: a newline first means the line has no assignment.
        let Some(eq_or_nl) = content.find(['=', '\n']) else {
            break;
        };
        if content.as_bytes()[eq_or_nl] == b'\n' {
            content = env_trim(&content[eq_or_nl + 1..]);
            continue;
        }
        // Key up to `=`.
        let mut key = env_trim(&content[..eq_or_nl]);
        content = &content[eq_or_nl + 1..];
        // `KEY=` (empty value).
        if content.is_empty() || content.as_bytes()[0] == b'\n' {
            pairs.insert(key.to_string(), String::new());
            continue;
        }
        content = env_trim(content);
        // Skip empty keys (`=value`, `"   "=value`).
        if key.is_empty() {
            continue;
        }
        // `export ` prefix.
        if let Some(rest) = key.strip_prefix("export ") {
            key = env_trim(rest);
        }
        if content.is_empty() {
            pairs.insert(key.to_string(), String::new());
            break;
        }
        let vfirst = content.as_bytes()[0];
        // Double-quoted value: expand literal `\n`, may span raw newlines.
        if vfirst == b'"' {
            if let Some(rel) = content[1..].find('"') {
                let closing = rel + 1;
                let value = content[1..closing].replace("\\n", "\n");
                pairs.insert(key.to_string(), value);
                match content[closing + 1..].find('\n') {
                    Some(nl) => content = &content[closing + 1 + nl + 1..],
                    None => content = "",
                }
                continue;
            }
            // No closing quote — fall through to the generic quote handler.
        }
        // Single / double / backtick quoted value (no escape expansion).
        if vfirst == b'\'' || vfirst == b'"' || vfirst == b'`' {
            match content[1..].find(vfirst as char) {
                None => match content.find('\n') {
                    Some(nl) => {
                        pairs.insert(key.to_string(), content[..nl].to_string());
                        content = &content[nl + 1..];
                    }
                    None => {
                        pairs.insert(key.to_string(), content.to_string());
                        break;
                    }
                },
                Some(rel) => {
                    let closing = rel + 1;
                    pairs.insert(key.to_string(), content[1..closing].to_string());
                    match content[closing + 1..].find('\n') {
                        Some(nl) => content = &content[closing + 1 + nl + 1..],
                        None => content = "",
                    }
                    continue;
                }
            }
        } else {
            // Unquoted value: up to the newline, `#` starts an inline comment, trim.
            let (raw, next) = match content.find('\n') {
                Some(nl) => (&content[..nl], &content[nl + 1..]),
                None => (content, ""),
            };
            let value = match raw.find('#') {
                Some(h) => &raw[..h],
                None => raw,
            };
            pairs.insert(key.to_string(), env_trim(value).to_string());
            content = next;
        }
        content = env_trim(content);
    }

    pairs.sort_keys();
    with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in pairs {
            let val = h.new_str(v);
            m.insert(k, val);
        }
        h.new_object(m)
    })
}

// ── MIMEType / MIMEParams ────────────────────────────────────────────────────
// Faithful port of Node's `lib/internal/mime.js`. A `MIMEType` is a native object
// tagged `@@native = "MIMEType"` with `type`/`subtype`/`essence` data properties
// and a `params` `MIMEParams` instance; a `MIMEParams` is tagged `@@native =
// "MIMEParams"` with its ordered unique `(name, value)` pairs in a hidden
// `@@pairs` array (mirrors the URLSearchParams representation).

/// An HTTP token code point (`NOT_HTTP_TOKEN_CODE_POINT` inverse).
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

/// An HTTP quoted-string code point (`NOT_HTTP_QUOTED_STRING_CODE_POINT` inverse):
/// tab, printable ASCII, and Latin-1 supplement.
fn is_quoted_string_char(c: char) -> bool {
    c == '\t' || ('\u{20}'..='\u{7e}').contains(&c) || ('\u{80}'..='\u{ff}').contains(&c)
}

/// HTTP whitespace (`\r \n \t space`).
fn is_http_ws(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\t' | ' ')
}

/// Lowercase only ASCII `A-Z`, leaving other code points intact (Node's
/// `toASCIILower`).
fn ascii_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Build the `TypeError [ERR_INVALID_MIME_SYNTAX]` message (with the offending
/// index when known).
fn mime_syntax_err(part: &str, s: &str, index: Option<usize>) -> String {
    match index {
        Some(i) => format!(
            "TypeError [ERR_INVALID_MIME_SYNTAX]: The MIME syntax for a {part} in \"{s}\" is invalid at {i}"
        ),
        None => format!(
            "TypeError [ERR_INVALID_MIME_SYNTAX]: The MIME syntax for a {part} in \"{s}\" is invalid"
        ),
    }
}

/// Parse `type/subtype` off the front of a MIME string; returns
/// `(type, subtype, remaining-params-string)`.
fn parse_type_and_subtype(s: &str) -> Result<(String, String, String), String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    // Skip leading HTTP whitespace.
    let mut pos = 0;
    while pos < n && is_http_ws(chars[pos]) {
        pos += 1;
    }
    // Read the type up to `/`.
    let type_end = (pos..n).find(|&i| chars[i] == '/');
    let trimmed_type: String = match type_end {
        Some(e) => chars[pos..e].iter().collect(),
        None => chars[pos..].iter().collect(),
    };
    let type_invalid = trimmed_type.chars().position(|c| !is_token_char(c));
    if trimmed_type.is_empty() || type_invalid.is_some() || type_end.is_none() {
        return Err(mime_syntax_err("type", s, type_invalid));
    }
    let type_end = type_end.unwrap();
    pos = type_end + 1;
    let mime_type = ascii_lower(&trimmed_type);
    // Read the subtype up to `;`.
    let sub_end = (pos..n).find(|&i| chars[i] == ';');
    let raw_subtype: &[char] = match sub_end {
        Some(e) => &chars[pos..e],
        None => &chars[pos..],
    };
    let mut new_pos = pos + raw_subtype.len();
    if sub_end.is_some() {
        new_pos += 1;
    }
    // Trim trailing HTTP whitespace from the subtype only.
    let mut end = raw_subtype.len();
    while end > 0 && is_http_ws(raw_subtype[end - 1]) {
        end -= 1;
    }
    let trimmed_subtype: String = raw_subtype[..end].iter().collect();
    let sub_invalid = trimmed_subtype.chars().position(|c| !is_token_char(c));
    if trimmed_subtype.is_empty() || sub_invalid.is_some() {
        return Err(mime_syntax_err("subtype", s, sub_invalid));
    }
    let subtype = ascii_lower(&trimmed_subtype);
    let params: String = chars[new_pos.min(n)..].iter().collect();
    Ok((mime_type, subtype, params))
}

/// Scan a quoted parameter value starting just after the opening `"` (Node's
/// `QUOTED_VALUE_PATTERN`). Returns `(matched_len, lone_backslash, closing_quote)`.
fn scan_quoted(chars: &[char], start: usize) -> (usize, bool, bool) {
    let n = chars.len();
    let mut i = start;
    let mut lone_backslash = false;
    let mut closing_quote = false;
    while i < n {
        match chars[i] {
            '\\' => {
                if i + 1 >= n {
                    lone_backslash = true;
                    i += 1;
                    break;
                }
                i += 2;
            }
            '"' => {
                closing_quote = true;
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    (i - start, lone_backslash, closing_quote)
}

/// Remove single `\` escapes (Node's `removeBackslashes`).
fn remove_backslashes(s: &[char]) -> String {
    let n = s.len();
    if n == 0 {
        return String::new();
    }
    let mut ret = String::new();
    let mut i = 0usize;
    while i < n - 1 {
        if s[i] == '\\' {
            i += 1;
            ret.push(s[i]);
        } else {
            ret.push(s[i]);
        }
        i += 1;
    }
    if i == n - 1 {
        ret.push(s[i]);
    }
    ret
}

/// Parse a MIME parameter string into ordered, unique `(name, value)` pairs.
fn parse_mime_params(s: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    // The source ends where trailing whitespace begins.
    let mut end_of_source = n;
    while end_of_source > 0 && is_http_ws(chars[end_of_source - 1]) {
        end_of_source -= 1;
    }
    let mut out: Vec<(String, String)> = Vec::new();
    let mut position = 0usize;
    while position < end_of_source {
        // Skip whitespace before the parameter name.
        while position < n && is_http_ws(chars[position]) {
            position += 1;
        }
        // Read the name up to `;`, `=`, or end.
        let mut after = position;
        while after < n && chars[after] != ';' && chars[after] != '=' {
            after += 1;
        }
        let name = ascii_lower(&chars[position..after].iter().collect::<String>());
        position = after;
        if position < end_of_source {
            let ch = chars[position];
            position += 1;
            // A `;` terminator means a value-less parameter — ignore it.
            if ch == ';' {
                continue;
            }
        }
        if position >= end_of_source {
            break;
        }
        let value = if chars[position] == '"' {
            // Quoted-string value.
            position += 1;
            let (matched_len, lone_backslash, closing_quote) = scan_quoted(&chars, position);
            let matched = &chars[position..position + matched_len];
            position += matched_len;
            let inside: &[char] = if lone_backslash || closing_quote {
                &matched[..matched.len().saturating_sub(1)]
            } else {
                matched
            };
            let mut v = remove_backslashes(inside);
            if lone_backslash {
                v.push('\\');
            }
            v
        } else {
            // Bare value up to `;`, trailing whitespace trimmed.
            let value_end = (position..n).find(|&i| chars[i] == ';').unwrap_or(n);
            let raw = &chars[position..value_end];
            position += raw.len();
            let mut end = raw.len();
            while end > 0 && is_http_ws(raw[end - 1]) {
                end -= 1;
            }
            let trimmed: String = raw[..end].iter().collect();
            if trimmed.is_empty() {
                // Node `continue`s here without the trailing `position++`, leaving
                // `position` on the `;` so the next iteration consumes it.
                continue;
            }
            trimmed
        };
        // Keep only valid, non-duplicate parameters (first value wins).
        let name_ok = !name.is_empty() && name.chars().all(is_token_char);
        let value_ok = value.chars().all(is_quoted_string_char);
        if name_ok && value_ok && !out.iter().any(|(k, _)| *k == name) {
            out.push((name, value));
        }
        position += 1;
    }
    out
}

/// Serialize a parameter value: bare if it is a valid token, else a quoted string
/// with `"`/`\` escaped (Node's `encode`).
fn encode_param_value(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if value.chars().all(is_token_char) {
        return value.to_string();
    }
    let mut escaped = String::with_capacity(value.len() + 2);
    for c in value.chars() {
        if c == '"' || c == '\\' {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("\"{escaped}\"")
}

/// Serialize ordered pairs as `name=value;name2=value2` (MIMEParams `toString`).
fn serialize_mime_params(pairs: &[(String, String)]) -> String {
    let mut ret = String::new();
    for (k, v) in pairs {
        if !ret.is_empty() {
            ret.push(';');
        }
        ret.push_str(k);
        ret.push('=');
        ret.push_str(&encode_param_value(v));
    }
    ret
}

/// Method names dispatched through `mime_params_instance_call` (for
/// `instance_has_method` wiring; `@@iterator` powers `for..of` / spread).
pub const MIME_PARAMS_METHODS: &[&str] = &[
    "get",
    "set",
    "has",
    "delete",
    "entries",
    "keys",
    "values",
    "toString",
    "toJSON",
    "@@iterator",
];

/// Method names dispatched through `mime_type_instance_call`.
pub const MIME_TYPE_METHODS: &[&str] = &["toString", "toJSON"];

/// Build a `MIMEParams` native object from ordered pairs.
fn make_mime_params(pairs: &[(String, String)]) -> Value {
    with_host(|h| {
        let items: Vec<Value> = pairs
            .iter()
            .map(|(k, v)| {
                let kv = vec![h.new_str(k.clone()), h.new_str(v.clone())];
                h.new_array(kv)
            })
            .collect();
        let arr = h.new_array(items);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("MIMEParams"));
        m.insert("@@pairs".into(), arr);
        h.new_object(m)
    })
}

/// Read the ordered `(name, value)` pairs out of a `MIMEParams`.
fn mime_pairs_of(recv: &Value) -> Vec<(String, String)> {
    with_host(|h| {
        let items: Vec<Value> = match h.get(recv) {
            Some(JsObj::Object(p)) => match p.get("@@pairs").and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => items.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        items
            .iter()
            .map(|it| match h.get(it) {
                Some(JsObj::Array(kv)) => {
                    let kv = kv.clone();
                    let k = kv.first().map(|x| h.str_of(x)).unwrap_or_default();
                    let v = kv.get(1).map(|x| h.str_of(x)).unwrap_or_default();
                    (k, v)
                }
                _ => (h.str_of(it), String::new()),
            })
            .collect()
    })
}

/// Overwrite a `MIMEParams`' backing `@@pairs` array.
fn set_mime_pairs(recv: &Value, pairs: &[(String, String)]) {
    with_host(|h| {
        let items: Vec<Value> = pairs
            .iter()
            .map(|(k, v)| {
                let kv = vec![h.new_str(k.clone()), h.new_str(v.clone())];
                h.new_array(kv)
            })
            .collect();
        let arr = h.new_array(items);
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@pairs".into(), arr);
        }
    });
}

/// `new util.MIMEParams()` — an empty parameter set (Node's constructor takes no
/// arguments).
pub fn construct_mime_params(_args: &[Value]) -> Result<Value, String> {
    Ok(make_mime_params(&[]))
}

/// `MIMEParams` instance methods.
pub fn mime_params_instance_call(
    recv: &Value,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    match method {
        "get" => {
            let name = super::arg_str(args, 0);
            match mime_pairs_of(recv).into_iter().find(|(k, _)| *k == name) {
                Some((_, v)) => Ok(with_host(|h| h.new_str(v))),
                None => Ok(with_host(|h| h.null())),
            }
        }
        "has" => {
            let name = super::arg_str(args, 0);
            Ok(Value::Bool(
                mime_pairs_of(recv).iter().any(|(k, _)| *k == name),
            ))
        }
        "set" => {
            let name = super::arg_str(args, 0);
            let value = super::arg_str(args, 1);
            if let Some(i) = name.chars().position(|c| !is_token_char(c)) {
                return Err(mime_syntax_err("parameter name", &name, Some(i)));
            }
            if name.is_empty() {
                return Err(mime_syntax_err("parameter name", &name, None));
            }
            if let Some(i) = value.chars().position(|c| !is_quoted_string_char(c)) {
                return Err(mime_syntax_err("parameter value", &value, Some(i)));
            }
            let mut pairs = mime_pairs_of(recv);
            match pairs.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 = value,
                None => pairs.push((name, value)),
            }
            set_mime_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "delete" => {
            let name = super::arg_str(args, 0);
            let mut pairs = mime_pairs_of(recv);
            pairs.retain(|(k, _)| *k != name);
            set_mime_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "keys" => {
            let pairs = mime_pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(k, _)| h.new_str(k)).collect();
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                    brand: Some("Generator"),
                })
            }))
        }
        "values" => {
            let pairs = mime_pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(_, v)| h.new_str(v)).collect();
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                    brand: Some("Generator"),
                })
            }))
        }
        "entries" | "@@iterator" => {
            let pairs = mime_pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        let kv = vec![h.new_str(k), h.new_str(v)];
                        h.new_array(kv)
                    })
                    .collect();
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                    brand: Some("Generator"),
                })
            }))
        }
        "toString" | "toJSON" => {
            let s = serialize_mime_params(&mime_pairs_of(recv));
            Ok(with_host(|h| h.new_str(s)))
        }
        _ => Err(crate::host::type_error(&format!(
            "mimeParams.{method} is not a function"
        ))),
    }
}

/// `new util.MIMEType(input)` — parse into `type`/`subtype`/`essence`/`params`.
pub fn construct_mime_type(args: &[Value]) -> Result<Value, String> {
    // Node coerces the argument to a string first (`${string}`), so a missing
    // argument parses as the literal `"undefined"` (which then throws).
    let input = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
    let (mime_type, subtype, params_str) = parse_type_and_subtype(&input)?;
    let essence = format!("{mime_type}/{subtype}");
    // Build the `MIMEParams` instance BEFORE the allocating `with_host` (never nest).
    let params = make_mime_params(&parse_mime_params(&params_str));
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("MIMEType"));
        m.insert("type".into(), h.new_str(mime_type));
        m.insert("subtype".into(), h.new_str(subtype));
        m.insert("essence".into(), h.new_str(essence));
        m.insert("params".into(), params);
        h.new_object(m)
    }))
}

/// `MIMEType` instance methods. `type`/`subtype`/`essence`/`params` are data
/// properties read directly; `toString`/`toJSON` serialize live (reflecting any
/// `params` mutation).
pub fn mime_type_instance_call(
    recv: &Value,
    method: &str,
    _args: &[Value],
) -> Result<Value, String> {
    match method {
        "toString" | "toJSON" => {
            // Read the essence and the live params object under one borrow.
            let (essence, params) = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => (
                    p.get("essence").map(|x| h.str_of(x)).unwrap_or_default(),
                    p.get("params").cloned().unwrap_or(Value::Undef),
                ),
                _ => (String::new(), Value::Undef),
            });
            let param_str = serialize_mime_params(&mime_pairs_of(&params));
            let out = if param_str.is_empty() {
                essence
            } else {
                format!("{essence};{param_str}")
            };
            Ok(with_host(|h| h.new_str(out)))
        }
        _ => Err(crate::host::type_error(&format!(
            "mimeType.{method} is not a function"
        ))),
    }
}
