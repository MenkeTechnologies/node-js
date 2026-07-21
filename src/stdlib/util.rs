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
        "inspect" => {
            // Honor an options `{ depth: N | null }` (default 2; null = unlimited).
            let depth = match args.get(1) {
                Some(opts) => match crate::builtins::get_property(opts, "depth") {
                    Ok(Value::Undef) => 2,
                    Ok(v) if with_host(|h| h.is_null(&v)) => usize::MAX,
                    Ok(v) => {
                        let n = with_host(|h| h.to_number(&v));
                        if n.is_finite() && n >= 0.0 {
                            n as usize
                        } else {
                            usize::MAX
                        }
                    }
                    Err(_) => 2,
                },
                None => 2,
            };
            crate::host::set_inspect_max_depth(depth);
            let out = with_host(|h| {
                let s = h.inspect(&args.first().cloned().unwrap_or(Value::Undef));
                h.new_str(s)
            });
            crate::host::set_inspect_max_depth(2);
            Ok(out)
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
        "debuglog" => return Some(debuglog(args)),
        "promisify" => return Some(promisify(args)),
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
    if !matches!(args[0], Value::Str(_))
        && !with_host(|h| matches!(h.get(&args[0]), Some(JsObj::Str(_))))
    {
        return with_host(|h| {
            args.iter()
                .map(|a| h.console_format(a))
                .collect::<Vec<_>>()
                .join(" ")
        });
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
                out.push_str(&if n.is_nan() {
                    "NaN".into()
                } else {
                    (n.trunc() as i64).to_string()
                });
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

// ── promisify / callbackify ──────────────────────────────────────────────────
// These build a REAL JS closure by compiling a factory expression and invoking it
// with the wrapped function — the same re-entrant nested-run path `vm.runInThisContext`
// uses — so the returned value is an ordinary user function (`typeof === "function"`).

/// Compile a single JS expression and run it on the current host, returning its
/// completion value. Re-entrant-safe (mirrors `vm::run_code`).
fn run_completion(src: &str) -> Result<Value, String> {
    let prog = crate::compile_completion(src)?;
    let chunk = crate::load_merged(prog);
    crate::host::run_chunk_on(chunk)
}

const PROMISIFY_SRC: &str = "(function(original){\n\
  return function(){\n\
    var self = this;\n\
    var args = Array.prototype.slice.call(arguments);\n\
    return new Promise(function(resolve, reject){\n\
      args.push(function(err, value){ if (err) reject(err); else resolve(value); });\n\
      original.apply(self, args);\n\
    });\n\
  };\n\
})";

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
fn promisify(args: &[Value]) -> Result<Value, String> {
    let orig = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &orig)) {
        return Err(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"original\" argument must be of type function"
                .into(),
        );
    }
    let factory = run_completion(PROMISIFY_SRC)?;
    crate::host::invoke(&factory, vec![orig], None)
}

/// `util.callbackify(fn)` → a function taking a trailing `(err, value)` callback,
/// invoked from the async function's resolved/rejected result.
fn callbackify(args: &[Value]) -> Result<Value, String> {
    let orig = args.first().cloned().unwrap_or(Value::Undef);
    if !with_host(|h| crate::host::is_callable(h, &orig)) {
        return Err(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"original\" argument must be of type function"
                .into(),
        );
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
fn strip_vt(s: &str) -> String {
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
fn style_codes(name: &str) -> Option<(u16, u16)> {
    Some(match name {
        "reset" => (0, 0),
        "bold" => (1, 22),
        "dim" => (2, 22),
        "italic" => (3, 23),
        "underline" => (4, 24),
        "blink" => (5, 25),
        "inverse" => (7, 27),
        "hidden" => (8, 28),
        "strikethrough" => (9, 29),
        "doubleunderline" => (21, 24),
        "overline" => (53, 55),
        "black" => (30, 39),
        "red" => (31, 39),
        "green" => (32, 39),
        "yellow" => (33, 39),
        "blue" => (34, 39),
        "magenta" => (35, 39),
        "cyan" => (36, 39),
        "white" => (37, 39),
        "gray" | "grey" | "blackBright" => (90, 39),
        "redBright" => (91, 39),
        "greenBright" => (92, 39),
        "yellowBright" => (93, 39),
        "blueBright" => (94, 39),
        "magentaBright" => (95, 39),
        "cyanBright" => (96, 39),
        "whiteBright" => (97, 39),
        "bgBlack" => (40, 49),
        "bgRed" => (41, 49),
        "bgGreen" => (42, 49),
        "bgYellow" => (43, 49),
        "bgBlue" => (44, 49),
        "bgMagenta" => (45, 49),
        "bgCyan" => (46, 49),
        "bgWhite" => (47, 49),
        "bgGray" | "bgGrey" | "bgBlackBright" => (100, 49),
        "bgRedBright" => (101, 49),
        "bgGreenBright" => (102, 49),
        "bgYellowBright" => (103, 49),
        "bgBlueBright" => (104, 49),
        "bgMagentaBright" => (105, 49),
        "bgCyanBright" => (106, 49),
        "bgWhiteBright" => (107, 49),
        _ => return None,
    })
}

/// `util.styleText(format, text)` — wrap `text` in the SGR codes for `format`
/// (a color/modifier name or an array of them). `"none"` is a no-op passthrough.
fn style_text(args: &[Value]) -> Result<Value, String> {
    let fmt = args.first().cloned().unwrap_or(Value::Undef);
    let names = style_names(&fmt);
    let mut result = super::arg_str(args, 1);
    for name in &names {
        if name == "none" {
            continue;
        }
        let (open, close) = style_codes(name).ok_or_else(|| {
            format!(
                "TypeError [ERR_INVALID_ARG_VALUE]: The argument 'format' must be a valid \
                 util.inspect.colors key. Received '{name}'"
            )
        })?;
        result = format!("\u{1b}[{open}m{result}\u{1b}[{close}m");
    }
    Ok(with_host(|h| h.new_str(result)))
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

fn errno_name(e: i32) -> Option<&'static str> {
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

/// A parsed option value (or list, when `multiple` is set).
enum Slot {
    Bool(bool),
    Str(String),
    ListBool(Vec<bool>),
    ListStr(Vec<String>),
}

/// The declared config for one option.
struct OptCfg {
    long: String,
    is_string: bool,
    multiple: bool,
    short: Option<String>,
    default: Option<Value>,
}

/// `util.parseArgs(config)` → `{ values, positionals }`. Implements the documented
/// short/long/`--`/`=`/grouped-short algorithm. `config.tokens` is not emitted.
fn parse_args(config_args: &[Value]) -> Result<Value, String> {
    let config = config_args.first().cloned().unwrap_or(Value::Undef);
    let tokens = read_arg_tokens(&config);
    let strict = read_bool_prop(&config, "strict", true);
    let allow_positionals = read_bool_prop(&config, "allowPositionals", false);
    let allow_negative = read_bool_prop(&config, "allowNegative", false);
    let opts = read_options(&config);

    let lookup_long = |name: &str| opts.iter().find(|o| o.long == name);
    let lookup_short = |c: &str| opts.iter().find(|o| o.short.as_deref() == Some(c));
    let is_bool_long = |name: &str| opts.iter().any(|o| o.long == name && !o.is_string);

    let mut values: IndexMap<String, Slot> = IndexMap::new();
    let mut positionals: Vec<String> = Vec::new();

    let mut i = 0usize;
    while i < tokens.len() {
        let tok = tokens[i].clone();
        if tok == "--" {
            for t in &tokens[i + 1..] {
                positionals.push(t.clone());
            }
            break;
        }
        if let Some(rest) = tok.strip_prefix("--") {
            let (raw_name, inline) = match rest.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (rest.to_string(), None),
            };
            // `--no-foo` negation for boolean options.
            let (name, negate) = match raw_name.strip_prefix("no-") {
                Some(base) if allow_negative && is_bool_long(base) => (base.to_string(), true),
                _ => (raw_name, false),
            };
            match lookup_long(&name) {
                None if strict => {
                    return Err(format!(
                        "Error [ERR_PARSE_ARGS_UNKNOWN_OPTION]: Unknown option '--{name}'"
                    ))
                }
                None => {
                    // Lenient: record as a boolean flag.
                    store(&mut values, &name, Slot::Bool(true), false);
                }
                Some(cfg) if cfg.is_string => {
                    let val = match inline {
                        Some(v) => v,
                        None => {
                            i += 1;
                            tokens.get(i).cloned().ok_or_else(|| {
                                format!(
                                    "Error [ERR_PARSE_ARGS_INVALID_OPTION_VALUE]: \
                                     Option '--{name} <value>' argument missing"
                                )
                            })?
                        }
                    };
                    store(&mut values, &cfg.long, Slot::Str(val), cfg.multiple);
                }
                Some(cfg) => {
                    if inline.is_some() && strict {
                        return Err(format!(
                            "Error [ERR_PARSE_ARGS_INVALID_OPTION_VALUE]: \
                             Option '--{}' does not take an argument",
                            cfg.long
                        ));
                    }
                    store(&mut values, &cfg.long, Slot::Bool(!negate), cfg.multiple);
                }
            }
        } else if tok.len() > 1 && tok.starts_with('-') {
            let chars: Vec<char> = tok[1..].chars().collect();
            let mut ci = 0usize;
            while ci < chars.len() {
                let short = chars[ci].to_string();
                match lookup_short(&short) {
                    None if strict => {
                        return Err(format!(
                            "Error [ERR_PARSE_ARGS_UNKNOWN_OPTION]: Unknown option '-{short}'"
                        ))
                    }
                    None => {
                        store(&mut values, &short, Slot::Bool(true), false);
                        ci += 1;
                    }
                    Some(cfg) if cfg.is_string => {
                        let remainder: String = chars[ci + 1..].iter().collect();
                        let val = if !remainder.is_empty() {
                            remainder
                        } else {
                            i += 1;
                            tokens.get(i).cloned().ok_or_else(|| {
                                format!(
                                    "Error [ERR_PARSE_ARGS_INVALID_OPTION_VALUE]: \
                                     Option '-{short}, --{} <value>' argument missing",
                                    cfg.long
                                )
                            })?
                        };
                        store(&mut values, &cfg.long, Slot::Str(val), cfg.multiple);
                        break;
                    }
                    Some(cfg) => {
                        store(&mut values, &cfg.long, Slot::Bool(true), cfg.multiple);
                        ci += 1;
                    }
                }
            }
        } else {
            if !allow_positionals && strict {
                return Err(format!(
                    "Error [ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL]: \
                     Unexpected argument '{tok}'. This command does not take positional arguments"
                ));
            }
            positionals.push(tok);
        }
        i += 1;
    }

    // Apply declared defaults for options that were never provided.
    for cfg in &opts {
        if !values.contains_key(&cfg.long) {
            if let Some(def) = &cfg.default {
                store_default(&mut values, cfg, def.clone());
            }
        }
    }

    Ok(build_parse_result(values, positionals))
}

/// Insert/append a parsed value under `name`, honoring `multiple`.
fn store(values: &mut IndexMap<String, Slot>, name: &str, slot: Slot, multiple: bool) {
    if !multiple {
        values.insert(name.to_string(), slot);
        return;
    }
    match values.get_mut(name) {
        Some(Slot::ListBool(v)) => {
            if let Slot::Bool(b) = slot {
                v.push(b);
            }
        }
        Some(Slot::ListStr(v)) => {
            if let Slot::Str(s) = slot {
                v.push(s);
            }
        }
        _ => {
            let init = match slot {
                Slot::Bool(b) => Slot::ListBool(vec![b]),
                Slot::Str(s) => Slot::ListStr(vec![s]),
                other => other,
            };
            values.insert(name.to_string(), init);
        }
    }
}

/// Seed a default value (already a JS `Value`) for an unset option.
fn store_default(values: &mut IndexMap<String, Slot>, cfg: &OptCfg, def: Value) {
    // A `multiple` default is expected to be an array; a scalar default is stored
    // directly. We coerce through the option's declared type.
    let slot = with_host(|h| match h.get(&def) {
        Some(JsObj::Array(items)) => {
            if cfg.is_string {
                Slot::ListStr(items.iter().map(|v| h.str_of(v)).collect())
            } else {
                Slot::ListBool(items.iter().map(|v| h.truthy(v)).collect())
            }
        }
        _ if cfg.is_string => Slot::Str(h.str_of(&def)),
        _ => Slot::Bool(h.truthy(&def)),
    });
    values.insert(cfg.long.clone(), slot);
}

/// Materialize `{ values, positionals }` in a single host borrow.
fn build_parse_result(values: IndexMap<String, Slot>, positionals: Vec<String>) -> Value {
    with_host(|h| {
        let mut vobj = IndexMap::new();
        for (k, slot) in values {
            let v = match slot {
                Slot::Bool(b) => Value::Bool(b),
                Slot::Str(s) => h.new_str(s),
                Slot::ListBool(items) => {
                    let arr = items.into_iter().map(Value::Bool).collect();
                    h.new_array(arr)
                }
                Slot::ListStr(items) => {
                    let arr = items.into_iter().map(|s| h.new_str(s)).collect();
                    h.new_array(arr)
                }
            };
            vobj.insert(k, v);
        }
        let values_v = h.new_object(vobj);
        let pos: Vec<Value> = positionals.into_iter().map(|s| h.new_str(s)).collect();
        let positionals_v = h.new_array(pos);
        let mut out = IndexMap::new();
        out.insert("values".to_string(), values_v);
        out.insert("positionals".to_string(), positionals_v);
        h.new_object(out)
    })
}

/// `config.args` as strings, or `process.argv.slice(2)` (the runtime's own tail).
fn read_arg_tokens(config: &Value) -> Vec<String> {
    let arr = crate::builtins::get_property(config, "args").unwrap_or(Value::Undef);
    let from_config = with_host(|h| match h.get(&arr) {
        Some(JsObj::Array(items)) => Some(items.iter().map(|v| h.str_of(v)).collect::<Vec<_>>()),
        _ => None,
    });
    from_config.unwrap_or_else(|| std::env::args().skip(2).collect())
}

/// Read a boolean config property, defaulting when absent/undefined.
fn read_bool_prop(config: &Value, name: &str, default: bool) -> bool {
    match crate::builtins::get_property(config, name) {
        Ok(Value::Undef) | Err(_) => default,
        Ok(v) => with_host(|h| h.truthy(&v)),
    }
}

/// Read `config.options` into an ordered list of `OptCfg`.
fn read_options(config: &Value) -> Vec<OptCfg> {
    let options = crate::builtins::get_property(config, "options").unwrap_or(Value::Undef);
    let keys: Vec<String> = with_host(|h| match h.get(&options) {
        Some(JsObj::Object(m)) => m.keys().filter(|k| !k.starts_with("@@")).cloned().collect(),
        _ => Vec::new(),
    });
    keys.into_iter()
        .map(|long| {
            let spec = crate::builtins::get_property(&options, &long).unwrap_or(Value::Undef);
            let type_str = crate::builtins::get_property(&spec, "type")
                .ok()
                .map(|v| with_host(|h| h.str_of(&v)))
                .unwrap_or_default();
            let short = match crate::builtins::get_property(&spec, "short") {
                Ok(Value::Undef) | Err(_) => None,
                Ok(v) => Some(with_host(|h| h.str_of(&v))),
            };
            let multiple = read_bool_prop(&spec, "multiple", false);
            let default = match crate::builtins::get_property(&spec, "default") {
                Ok(Value::Undef) | Err(_) => None,
                Ok(v) => Some(v),
            };
            OptCfg {
                long,
                is_string: type_str == "string",
                multiple,
                short,
                default,
            }
        })
        .collect()
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
                h.alloc(JsObj::Iter { items, idx: 0 })
            }))
        }
        "values" => {
            let pairs = mime_pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(_, v)| h.new_str(v)).collect();
                h.alloc(JsObj::Iter { items, idx: 0 })
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
                h.alloc(JsObj::Iter { items, idx: 0 })
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
