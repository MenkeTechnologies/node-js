//! Node.js core modules implemented natively for node-js.
//!
//! A `require(spec)` (see `builtins::call_builtin_function`) resolves a supported
//! module to a `JsObj::Builtin("<module>")` namespace value — exactly the shape
//! of the built-in `console`/`Math` namespaces — so `mod.method(...)` dispatches
//! through `host::call_method` → `builtins::call_builtin_function("<module>.<method>")`
//! → `stdlib::call`, and `const { method } = require('mod')` reads the method as a
//! first-class `Builtin("mod.method")` via `namespace_property`.
//!
//! Every stdlib function is free-standing and acquires the thread-local `JsHost`
//! through `with_host` only around allocations (and releases it before any
//! re-entrant `host::invoke`), so callbacks (`fs` async, `EventEmitter.emit`,
//! `assert.throws`) never double-borrow the host. Stateful instances (`Buffer`,
//! crypto `Hash`, `EventEmitter`, `URL`) are plain objects carrying a hidden
//! `@@native` tag (filtered from enumeration/display like `@@iterator`); their
//! methods route through `instance_call` from `host::call_method`.

use crate::host::{with_host, JsObj};
use fusevm::Value;

pub mod assert;
pub mod async_hooks;
pub mod buffer;
pub mod child_process;
pub mod cluster;
pub mod console;
pub mod crypto;
pub mod date;
pub mod dgram;
pub mod diagnostics_channel;
pub mod dns;
pub mod domain;
pub mod events;
pub mod fetch;
pub mod fs;
pub mod fs_promises;
pub mod http;
pub mod http2;
pub mod https;
pub mod net;
pub mod node_module;
pub mod os;
pub mod path;
pub mod perf_hooks;
pub mod process;
pub mod punycode;
pub mod querystring;
pub mod readline;
pub mod repl;
pub mod stream;
pub mod stream_consumers;
pub mod stream_promises;
pub mod stream_web;
pub mod string_decoder;
pub mod timers;
pub mod tls;
pub mod trace_events;
pub mod tty;
pub mod typedarray;
pub mod url;
pub mod url_legacy;
pub mod util;
pub mod util_types;
pub mod v8;
pub mod vm;
pub mod worker_threads;
pub mod zlib;

/// Native-heavy core modules that node-js does not yet implement (TLS handshakes,
/// HTTP/2 framing, OS worker threads sharing the thread-local heap, UDP sockets,
/// V8 inspector, etc.). `require`ing them succeeds and yields a namespace so that
/// programs which import-then-conditionally-use them still load; ACTUALLY calling
/// a method throws `Error: <mod>.<method> is not implemented in node-js`. This is
/// an honest not-yet-built surface, never a silent fake.
pub const UNIMPLEMENTED_MODULES: &[&str] = &["inspector", "wasi"];

/// True if `ns` is a known-but-unimplemented core module (see `UNIMPLEMENTED_MODULES`).
pub fn is_unimplemented(ns: &str) -> bool {
    UNIMPLEMENTED_MODULES.contains(&ns)
}

/// Canonical namespace name a `require(spec)` resolves to (after stripping an
/// optional `node:` prefix), or `None` for an unsupported module.
pub fn resolve(spec: &str) -> Option<&'static str> {
    match spec.strip_prefix("node:").unwrap_or(spec) {
        "fs" => Some("fs"),
        "path" => Some("path"),
        "os" => Some("os"),
        "util" => Some("util"),
        "assert" => Some("assert"),
        "crypto" => Some("crypto"),
        "buffer" => Some("buffer"),
        "url" => Some("url"),
        "process" => Some("process"),
        "net" => Some("net"),
        "http" => Some("http"),
        "stream" => Some("stream"),
        "tty" => Some("tty"),
        // The `events` module's export IS the EventEmitter constructor, so
        // `require('events')` yields the ctor namespace directly.
        "events" => Some("EventEmitter"),
        "string_decoder" => Some("string_decoder"),
        "zlib" => Some("zlib"),
        "querystring" => Some("querystring"),
        "console" => Some("console"),
        // `path/posix` is exactly our POSIX `path` (node-js targets a POSIX host,
        // so `require('path') === path.posix`); `path/win32` is the separate
        // backslash flavor. `assert/strict` is `assert` (already strict-based).
        "path/posix" => Some("path"),
        "path/win32" => Some("path/win32"),
        // `sys` is the long-deprecated alias for `util`.
        "sys" => Some("util"),
        // `require('assert/strict')` IS the strict namespace, so its `equal`
        // and `deepEqual` are the strict comparisons. Pointing it at the plain
        // `assert` made `require('assert/strict').equal(1, '1')` pass. The
        // strict namespace already existed for `assert.strict.*`.
        "assert/strict" => Some("assertStrict"),
        "child_process" => Some("child_process"),
        "dns" => Some("dns"),
        "punycode" => Some("punycode"),
        "timers" => Some("timers"),
        "timers/promises" => Some("timers/promises"),
        "perf_hooks" => Some("perf_hooks"),
        "async_hooks" => Some("async_hooks"),
        "util/types" => Some("util/types"),
        "diagnostics_channel" => Some("diagnostics_channel"),
        "v8" => Some("v8"),
        "readline" => Some("readline"),
        "vm" => Some("vm"),
        "fs/promises" => Some("fs/promises"),
        "dgram" => Some("dgram"),
        "dns/promises" => Some("dns/promises"),
        "worker_threads" => Some("worker_threads"),
        "tls" => Some("tls"),
        "https" => Some("https"),
        "repl" => Some("repl"),
        "cluster" => Some("cluster"),
        "domain" => Some("domain"),
        "http2" => Some("http2"),
        "trace_events" => Some("trace_events"),
        "module" => Some("module"),
        "stream/consumers" => Some("stream/consumers"),
        "stream/promises" => Some("stream/promises"),
        "stream/web" => Some("stream/web"),
        other => UNIMPLEMENTED_MODULES.iter().copied().find(|&m| m == other),
    }
}

/// True if `qualified` (`namespace.method`) is a stdlib method that
/// `call_builtin_function` should route into `call` (extends `is_known_builtin`).
pub fn is_method(qualified: &str) -> bool {
    let Some((ns, m)) = qualified.split_once('.') else {
        return qualified == "assert";
    };
    // Any method on an unimplemented namespace routes to `call`, which throws an
    // honest "not implemented" error (so `mod.foo()` fails clearly rather than
    // silently returning undefined).
    is_unimplemented(ns) || namespace_methods(ns).contains(&m) || namespace_ctors(ns).contains(&m)
}

/// The callable members of builtin namespace `ns`. THE single table backing both
/// `is_method` (does `ns.m` dispatch?) and `namespace_keys` (what does `for (k in
/// ns)` yield?), so a method can never be callable-but-unenumerable or the reverse.
pub fn namespace_methods(ns: &str) -> &'static [&'static str] {
    match ns {
        "fs" => fs::METHODS,
        "path" | "path/win32" => path::METHODS,
        "os" => os::METHODS,
        "util" => util::METHODS,
        "assert" | "assertStrict" => assert::METHODS,
        "crypto" => crypto::METHODS,
        "Buffer" => buffer::STATIC_METHODS,
        "buffer" => buffer::MODULE_METHODS,
        "Date" => date::STATIC_METHODS,
        "Response" => fetch::RESPONSE_STATICS,
        "AbortSignal" => fetch::ABORT_SIGNAL_STATICS,
        n if typedarray::is_ctor(n) => typedarray::STATIC_METHODS,
        "url" => url::MODULE_METHODS,
        "net" => net::MODULE_METHODS,
        "http" => http::MODULE_METHODS,
        "stream" => stream::METHODS,
        n if stream::is_class(n) => stream::STATIC_METHODS,
        "worker_threads" => worker_threads::METHODS,
        "zlib" => zlib::MODULE_METHODS,
        "querystring" => querystring::METHODS,
        "tty" => tty::METHODS,
        "process" => process::METHODS,
        "EventEmitter" => events::STATIC_METHODS,
        "console" => console::METHODS,
        "child_process" => child_process::METHODS,
        "dns" => dns::METHODS,
        "dns/promises" => dns::PROMISES_METHODS,
        "punycode" => punycode::METHODS,
        "timers" => timers::METHODS,
        "timers/promises" => timers::PROMISES_METHODS,
        "perf_hooks" | "performance" => perf_hooks::METHODS,
        "async_hooks" => async_hooks::METHODS,
        "AsyncResource" => async_hooks::RESOURCE_STATIC_METHODS,
        "util/types" => util_types::METHODS,
        "diagnostics_channel" => diagnostics_channel::METHODS,
        "v8" => v8::METHODS,
        "readline" => readline::METHODS,
        "vm" => vm::METHODS,
        "fs/promises" => fs_promises::METHODS,
        "dgram" => dgram::MODULE_METHODS,
        "tls" => tls::MODULE_METHODS,
        "https" => https::MODULE_METHODS,
        "repl" => repl::METHODS,
        "cluster" => cluster::METHODS,
        "domain" => domain::METHODS,
        "http2" => http2::METHODS,
        "trace_events" => trace_events::METHODS,
        "module" => node_module::METHODS,
        "Module" => node_module::MODULE_STATIC_METHODS,
        "stream/consumers" => stream_consumers::METHODS,
        "stream/promises" => stream_promises::METHODS,
        _ => &[],
    }
}

/// Class/constructor members a namespace re-exports as values rather than
/// callable methods (`require('buffer').Buffer`, `require('url').URL`). They are
/// enumerable own keys too, so `for (k in buffer)` sees `Buffer`.
pub fn namespace_ctors(ns: &str) -> &'static [&'static str] {
    match ns {
        "buffer" => &["Buffer", "Blob", "File"],
        "url" => &["URL", "URLSearchParams"],
        "EventEmitter" => &["EventEmitter"],
        "async_hooks" => &["AsyncLocalStorage", "AsyncResource"],
        "string_decoder" => &["StringDecoder"],
        "assert" => &["AssertionError"],
        "console" => &["Console"],
        "vm" => &["Script"],
        "fs" => &["promises"],
        // `stream/web` exports nothing BUT classes (`METHODS` is empty), so
        // without this arm the namespace had no enumerable key at all: measured
        // against node v26.7.0, `Object.keys(require('stream/web')).length` was
        // 0 here and 18 there, even though every one of the classes resolved
        // fine through `constant`. A namespace that answers property reads but
        // enumerates empty breaks the copy-the-module pattern
        // (`{ ...require('stream/web') }`, `for (k in web)`).
        "stream/web" => stream_web::CLASSES,
        _ => &[],
    }
}

/// The enumerable own keys of the builtin namespace `ns` — what `for (key in ns)`
/// and `Object.keys(ns)` yield. These are the members node-js ACTUALLY
/// implements, not Node's full export list, so a package that copies a namespace
/// key-by-key (safer-buffer clones `buffer` and `Buffer`) ends up with exactly the
/// working set rather than an empty object.
pub fn namespace_keys(ns: &str) -> Vec<String> {
    // The `require.cache` view enumerates the resolved filenames it holds.
    if ns == crate::builtins::REQUIRE_CACHE {
        return crate::module::cache_keys();
    }
    let mut out: Vec<String> = namespace_ctors(ns).iter().map(|s| s.to_string()).collect();
    for m in namespace_methods(ns) {
        if !out.iter().any(|k| k == m) {
            out.push((*m).to_string());
        }
    }
    out
}

/// Dispatch a resolved stdlib builtin (`assert`, or `namespace.method`). Returns
/// `None` if `name` is not a stdlib builtin (the caller falls through to the core
/// builtin table).
pub fn call(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if name == "assert" {
        return Some(assert::assert_ok(args));
    }
    let (ns, m) = name.split_once('.')?;
    Some(match ns {
        "fs" => fs::call(m, args)?,
        "path" => path::call(path::Flavor::Posix, m, args)?,
        "path/win32" => path::call(path::Flavor::Win32, m, args)?,
        "os" => os::call(m, args)?,
        "util" => util::call(m, args)?,
        "assert" => assert::call(m, args)?,
        "assertStrict" => assert::strict_call(m, args)?,
        "crypto" => crypto::call(m, args)?,
        "Buffer" => buffer::static_call(m, args)?,
        "buffer" if m == "Buffer" => Ok(with_host(|h| h.alloc(JsObj::Builtin("Buffer".into())))),
        "buffer" => buffer::module_call(m, args)?,
        "Date" => date::static_call(m, args)?,
        "Response" | "AbortSignal" => fetch::static_call(ns, m, args)?,
        n if typedarray::is_ctor(n) => typedarray::static_call(n, m, args)?,
        "url" if m == "URL" => Ok(with_host(|h| h.alloc(JsObj::Builtin("URL".into())))),
        "url" => url::call(m, args)?,
        "net" => net::call(m, args)?,
        "http" => http::call(m, args)?,
        "stream" => stream::call(m, args)?,
        "worker_threads" => worker_threads::call(m, args)?,
        "zlib" => zlib::call(m, args)?,
        "querystring" => querystring::call(m, args)?,
        "tty" => tty::call(m, args)?,
        "process" => process::call(m, args)?,
        "EventEmitter" if m == "EventEmitter" => Ok(with_host(|h| {
            h.alloc(JsObj::Builtin("EventEmitter".into()))
        })),
        "EventEmitter" => events::static_call(m, args)?,
        n if stream::is_class(n) => stream::static_call(n, m, args)?,
        "console" => console::call(m, args)?,
        "child_process" => child_process::call(m, args)?,
        "dns" => dns::call(m, args)?,
        "punycode" => punycode::call(m, args)?,
        "timers" => timers::call(m, args)?,
        "timers/promises" => timers::promises_call(m, args)?,
        "perf_hooks" | "performance" => perf_hooks::call(m, args)?,
        "async_hooks" => async_hooks::call(m, args)?,
        "AsyncResource" => async_hooks::static_call(m, args)?,
        "util/types" => util_types::call(m, args)?,
        "diagnostics_channel" => diagnostics_channel::call(m, args)?,
        "v8" => v8::call(m, args)?,
        "readline" => readline::call(m, args)?,
        "vm" => vm::call(m, args)?,
        "fs/promises" => fs_promises::call(m, args)?,
        "dgram" => dgram::call(m, args)?,
        // dns/promises: getServers/setServers/get|setDefaultResultOrder are shared
        // sync fns; every other method maps to dns's `promise<Cap>` variant.
        "dns/promises" => match m {
            "getServers" | "setServers" | "getDefaultResultOrder" | "setDefaultResultOrder" => {
                dns::call(m, args)?
            }
            _ => {
                let mut pm = String::from("promise");
                let mut cs = m.chars();
                if let Some(c) = cs.next() {
                    pm.extend(c.to_uppercase());
                    pm.push_str(cs.as_str());
                }
                dns::call(&pm, args)?
            }
        },
        "tls" => tls::call(m, args)?,
        "https" => https::call(m, args)?,
        "repl" => repl::call(m, args)?,
        "cluster" => cluster::call(m, args)?,
        "domain" => domain::call(m, args)?,
        "http2" => http2::call(m, args)?,
        "trace_events" => trace_events::call(m, args)?,
        "module" => node_module::call(m, args)?,
        "Module" => node_module::static_call(m, args)?,
        "stream/consumers" => stream_consumers::call(m, args)?,
        "stream/promises" => stream_promises::call(m, args)?,
        _ if is_unimplemented(ns) => Err(format!("Error: {ns}.{m} is not implemented in node-js")),
        _ => return None,
    })
}

/// A non-function constant on a stdlib namespace (`path.sep`, `os.EOL`,
/// `buffer.Buffer`, `url.URL`), reachable via `namespace_property`.
pub fn constant(ns: &str, name: &str) -> Option<Value> {
    match ns {
        // Both flavors carry `.posix`/`.win32` cross-links, exactly as Node's
        // `posix.win32 = win32.win32 = win32; posix.posix = win32.posix = posix`.
        "path" | "path/win32" if name == "posix" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("path".into()))))
        }
        "path" | "path/win32" if name == "win32" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("path/win32".into()))))
        }
        "path" => path::constant(path::Flavor::Posix, name),
        "path/win32" => path::constant(path::Flavor::Win32, name),
        "os" => os::constant(name),
        // `EventEmitter.defaultMaxListeners` is a DATA property, so it belongs
        // here rather than among the static methods (which would make it read
        // as a function). It was absent: node reports 10.
        "EventEmitter" | "events" if name == "defaultMaxListeners" => Some(Value::Float(10.0)),
        // `Buffer.poolSize` is a DATA property, not a method, so it belongs here
        // rather than in `STATIC_METHODS` (which would make it read as a
        // function). It was absent entirely: `Buffer.poolSize` was `undefined`
        // where node v26.7.0 reports 65536. node-js allocates each Buffer on its
        // own, so this is the documented constant, not a live allocator figure.
        "Buffer" if name == "poolSize" => Some(Value::Float(65536.0)),
        "buffer" if name == "Buffer" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("Buffer".into()))))
        }
        "buffer" if matches!(name, "Blob" | "File") => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        "url" if name == "URL" => Some(with_host(|h| h.alloc(JsObj::Builtin("URL".into())))),
        "net" => net::constant(name),
        "tty" => tty::constant(name),
        "repl" => repl::constant(name),
        "readline" => readline::constant(name),
        "diagnostics_channel" => diagnostics_channel::constant(name),
        "v8" => v8::constant(name),
        "console" if name == "Console" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("Console".into()))))
        }
        "assert" if name == "AssertionError" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("AssertionError".into()))
        })),
        "assert" if name == "strict" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("assertStrict".into()))
        })),
        "stream" => stream::constant(name),
        "http" => http::constant(name),
        "string_decoder" if name == "StringDecoder" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("StringDecoder".into()))
        })),
        "process" => process::constant(name),
        "EventEmitter" if name == "EventEmitter" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("EventEmitter".into()))
        })),
        "perf_hooks" | "performance" => perf_hooks::constant(name),
        "dns" => dns::constant(name),
        "punycode" => punycode::constant(name),
        "async_hooks" if matches!(name, "AsyncLocalStorage" | "AsyncResource") => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        "vm" if name == "Script" => Some(with_host(|h| h.alloc(JsObj::Builtin("Script".into())))),
        "url" if name == "URLSearchParams" => Some(with_host(|h| {
            h.alloc(JsObj::Builtin("URLSearchParams".into()))
        })),
        "fs" if name == "promises" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("fs/promises".into()))))
        }
        "worker_threads" => worker_threads::constant(name),
        "https" => https::constant(name),
        "cluster" => cluster::constant(name),
        "domain" => domain::constant(name),
        "http2" => http2::constant(name),
        "module" => node_module::constant(name),
        "Module" => node_module::static_constant(name),
        "stream/web" => stream_web::constant(name),
        // util.types / util.TextEncoder|TextDecoder / util.MIMEType|MIMEParams.
        "util" => util::constant(name),
        // crypto class-constructor exports (require('crypto').Sign etc.) — the
        // instances are made by factory fns, but the ctor names must resolve.
        "crypto"
            if matches!(
                name,
                "Sign"
                    | "Verify"
                    | "KeyObject"
                    | "DiffieHellman"
                    | "ECDH"
                    | "X509Certificate"
                    | "Hash"
                    | "Hmac"
                    | "Cipheriv"
                    | "Decipheriv"
            ) =>
        {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        _ => None,
    }
}

/// Construct a stdlib class instance (`new URL(...)`, `new EventEmitter()`, and
/// `new Buffer(...)` legacy), reachable from `construct_builtin`. `None` if `name`
/// is not a stdlib constructor.
pub fn construct(name: &str, args: &[Value]) -> Option<Result<Value, String>> {
    match name {
        "URL" => Some(url::construct(args)),
        "EventEmitter" => Some(Ok(events::new_emitter())),
        // `new Buffer(x)` and the deprecated call form `Buffer(x)` are the same
        // operation, and it is NOT simply `Buffer.from`: a NUMBER allocates that
        // many zero bytes, where `Buffer.from(3)` is a TypeError in Node.
        // Measured on node v26.7.0, `new Buffer(3)` and `Buffer(3)` are both
        // `<Buffer 00 00 00>` (zero-filled since the `Buffer.alloc` semantics
        // landed), while `new Buffer('ab')` and `new Buffer([1,2])` behave as
        // `from`. Routing everything through `from` made `new Buffer(3)` one byte
        // long.
        "Buffer" => {
            let numeric = matches!(args.first(), Some(Value::Int(_)) | Some(Value::Float(_)))
                && args.len() == 1;
            let m = if numeric { "alloc" } else { "from" };
            Some(buffer::static_call(m, args).unwrap_or(Ok(Value::Undef)))
        }
        "Date" => Some(date::construct(args)),
        "StringDecoder" => Some(string_decoder::construct(args)),
        "WeakRef" => Some(typedarray::construct_weakref(args)),
        "FinalizationRegistry" => Some(typedarray::construct_finalization_registry(args)),
        n if fetch::is_class(n) => fetch::construct(n, args),
        "TextEncoder" => Some(typedarray::construct_text_encoder()),
        "TextDecoder" => Some(typedarray::construct_text_decoder(args)),
        n if typedarray::is_ctor(n) => Some(typedarray::construct(n, args)),
        n if stream::is_class(n) => Some(Ok(stream::construct(n, args))),
        "AsyncLocalStorage" | "AsyncResource" => async_hooks::construct(name, args),
        "Script" => Some(vm::construct(args)),
        "URLSearchParams" => Some(url::construct_search_params(args)),
        "Worker" => Some(worker_threads::construct_worker(args)),
        "Domain" => Some(domain::construct(args)),
        "Tracing" => Some(trace_events::construct(args)),
        "Blob" => Some(buffer::construct_blob(args)),
        "File" => Some(buffer::construct_file(args)),
        "AssertionError" => Some(Ok(assert::construct_assertion_error(args))),
        "X509Certificate" => Some(crypto::construct_x509(args)),
        "MIMEType" => Some(util::construct_mime_type(args)),
        "MIMEParams" => Some(util::construct_mime_params(args)),
        "Resolver" => Some(Ok(dns::construct_resolver(args))),
        "ReadStream" | "WriteStream" => Some(Ok(tty::construct(name, args))),
        "MessageChannel" => Some(worker_threads::construct_message_channel(args)),
        "BroadcastChannel" => Some(worker_threads::construct_broadcast_channel(args)),
        "PerformanceObserver" => Some(perf_hooks::construct(name, args)),
        "REPLServer" | "Recoverable" => Some(repl::construct(name, args)),
        "Interface" => Some(readline::construct(args)),
        "Console" => Some(console::construct(args)),
        "Serializer" | "DefaultSerializer" | "Deserializer" | "DefaultDeserializer" => {
            Some(v8::construct(name, args))
        }
        // net/http constructors: their `construct` already returns Option<Result>.
        "Socket" | "Stream" | "Server" | "SocketAddress" | "BlockList" => {
            net::construct(name, args)
        }
        "Agent" | "http.Server" => http::construct(name, args),
        // stream/web WHATWG classes (its `construct` returns Option<Result>).
        n if stream_web::is_class(n) => stream_web::construct(n, args),
        _ => None,
    }
}

/// The hidden `@@native` instance tag of `recv` (`"Buffer"`/`"Hash"`/
/// `"EventEmitter"`/`"URL"`), or `None` for a non-native object.
pub fn native_tag(recv: &Value) -> Option<String> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@native").map(|v| h.str_of(v)),
        _ => None,
    })
}

/// Native instance tags whose `instance_call` implements `toJSON()`, which
/// `JSON.stringify` must invoke before serializing the value. (`instance_has_method`
/// only covers tags with a declared method table; `Date` dispatches directly.)
pub fn has_to_json(tag: &str) -> bool {
    matches!(tag, "Buffer" | "Date" | "URL" | "MIMEType" | "MIMEParams")
}

/// Whether `name` is a method of a native instance tagged `tag`. Used by
/// `get_property` so a method *read* (`server.listen.apply(...)`, the express
/// listen path) yields a bound method rather than `undefined` — the method is
/// still dispatched through `instance_call` when the bound method is invoked.
pub fn instance_has_method(tag: &str, name: &str) -> bool {
    let (base, emitter) = instance_method_lists(tag);
    base.contains(&name) || emitter.contains(&name)
}

/// The method names a native instance tagged `tag` carries, as
/// `(its own list, the EventEmitter surface it also gets or empty)`.
///
/// Split out of [`instance_has_method`] so the same table can be *enumerated*,
/// not only queried: `host::ensure_ctor_proto` builds a native constructor's
/// real `.prototype` object from it. A predicate alone would have forced a
/// second, hand-maintained list of the same names — the drift that put
/// `listeners` on nine dispatchers and not on the three that run.
pub fn instance_method_lists(tag: &str) -> (&'static [&'static str], &'static [&'static str]) {
    // Shared EventEmitter surface for the emitter-backed instances. Read from
    // `events::METHODS` so what an instance ADVERTISES here can never drift from
    // what the dispatchers actually delegate.
    const EMITTER: &[&str] = events::METHODS;
    let base: &[&str] = match tag {
        "Timeout" => timers::TIMEOUT_METHODS,
        "IntervalIterator" => timers::INTERVAL_METHODS,
        "Immediate" => timers::IMMEDIATE_METHODS,
        "Server" => &["listen", "close", "address"],
        "Socket" => &[
            "write",
            "end",
            "destroy",
            "pause",
            "resume",
            "setEncoding",
            "setKeepAlive",
            "setNoDelay",
            "setTimeout",
            "ref",
            "unref",
            "connect",
        ],
        "ServerResponse" => &[
            "writeHead",
            "setHeader",
            "getHeader",
            "getHeaderNames",
            "getHeaders",
            "hasHeader",
            "removeHeader",
            "write",
            "end",
            "flushHeaders",
        ],
        "IncomingMessage" => &["pause", "resume", "setEncoding", "destroy"],
        "Buffer" => buffer::INSTANCE_METHODS,
        "Date" => date::INSTANCE_METHODS,
        "Readable" | "Writable" | "Duplex" | "Transform" | "PassThrough" | "Stream" => &[
            "read",
            "write",
            "end",
            "pipe",
            "pause",
            "resume",
            "setEncoding",
            "destroy",
            "push",
        ],
        "URL" => &["toString", "toJSON"],
        "AsyncLocalStorage" => async_hooks::ALS_METHODS,
        "AsyncHook" => async_hooks::HOOK_METHODS,
        "AsyncResource" => async_hooks::RESOURCE_METHODS,
        "Channel" => &["subscribe", "unsubscribe", "publish"],
        "WriteStream" => &[
            "write",
            "end",
            "on",
            "once",
            "removeListener",
            "cork",
            "uncork",
            "setEncoding",
        ],
        // `Hash` and `Hmac` answer the same two methods (both route to
        // `crypto::hashlike_call`). Only `Hmac` was listed, so
        // `ensure_ctor_proto("Hash")` found nothing and `crypto.Hash.prototype`
        // read `undefined` — the ES5-subclassing hole this table exists to
        // close, still open for one of the two constructors it documents.
        "Hash" | "Hmac" => &["update", "digest"],
        "StringDecoder" => string_decoder::INSTANCE_METHODS,
        "Interface" => readline::INTERFACE_METHODS,
        "Script" => vm::SCRIPT_METHODS,
        "URLSearchParams" => url::SEARCH_PARAMS_METHODS,
        "UdpSocket" => dgram::SOCKET_METHODS,
        "Worker" => worker_threads::WORKER_METHODS,
        "MessagePort" => worker_threads::PORT_METHODS,
        "TLSServer" => tls::SERVER_METHODS,
        "TLSSocket" => tls::SOCKET_METHODS,
        "HTTPSServerResponse" => https::RESPONSE_METHODS,
        "HTTPSClientRequest" => https::CLIENT_REQUEST_METHODS,
        "REPLServer" => repl::REPLSERVER_METHODS,
        "ClusterWorker" => cluster::WORKER_METHODS,
        "Domain" => domain::DOMAIN_METHODS,
        "Tracing" => trace_events::TRACING_METHODS,
        "Http2Server" => http2::SERVER_METHODS,
        "Http2Stream" => http2::STREAM_METHODS,
        "Http2Session" => http2::SESSION_METHODS,
        "Cipheriv" | "Decipheriv" => &["update", "final", "setAutoPadding"],
        "BlockList" => net::BLOCKLIST_METHODS,
        "ClientRequest" => http::CLIENT_REQUEST_METHODS,
        "Agent" => &["destroy", "getName"],
        "Blob" | "File" => buffer::BLOB_METHODS,
        "ReadStream" => tty::READ_STREAM_METHODS,
        "Dirent" => fs::DIRENT_METHODS,
        "Dir" => fs::DIR_METHODS,
        "FSReadStream" => fs::READ_STREAM_METHODS,
        "FSWriteStream" => fs::WRITE_STREAM_METHODS,
        "Resolver" => dns::RESOLVER_METHODS,
        "Histogram" => perf_hooks::HISTOGRAM_METHODS,
        "PerformanceObserver" => perf_hooks::PERFORMANCE_OBSERVER_METHODS,
        "PerformanceObserverEntryList" => perf_hooks::OBSERVER_ENTRY_LIST_METHODS,
        "BroadcastChannel" => worker_threads::BROADCAST_CHANNEL_METHODS,
        "TracingChannel" => diagnostics_channel::TRACING_CHANNEL_METHODS,
        "Serializer" => v8::SERIALIZER_METHODS,
        "Deserializer" => v8::DESERIALIZER_METHODS,
        "Console" => console::CONSOLE_METHODS,
        "ChildProcess" => child_process::CHILD_PROCESS_METHODS,
        "Sign" => &["update", "sign"],
        "Verify" => &["update", "verify"],
        "KeyObject" => &["export", "equals"],
        "DiffieHellman" => &[
            "generateKeys",
            "computeSecret",
            "getPrime",
            "getGenerator",
            "getPublicKey",
            "getPrivateKey",
            "setPublicKey",
            "setPrivateKey",
        ],
        "ECDH" => &[
            "generateKeys",
            "computeSecret",
            "getPublicKey",
            "getPrivateKey",
            "setPrivateKey",
        ],
        "X509Certificate" => &["toString"],
        "FinalizationRegistry" => &["register", "unregister"],
        "MIMEType" => util::MIME_TYPE_METHODS,
        "MIMEParams" => util::MIME_PARAMS_METHODS,
        t if fetch::is_class(t) => fetch::methods_for(t),
        t if stream_web::is_class(t) => stream_web::methods_for(t),
        _ => &[],
    };
    let is_emitter = matches!(
        tag,
        "Server"
            | "Socket"
            | "ServerResponse"
            | "IncomingMessage"
            | "EventEmitter"
            | "Readable"
            | "Writable"
            | "Duplex"
            | "Transform"
            | "PassThrough"
            | "Stream"
            | "UdpSocket"
            | "Worker"
            | "MessagePort"
            | "TLSServer"
            | "TLSSocket"
            | "HTTPSServerResponse"
            | "HTTPSClientRequest"
            | "ClusterWorker"
            | "Domain"
            | "Http2Server"
            | "Http2Stream"
            | "Http2Session"
            | "ClientRequest"
            | "FSReadStream"
            | "FSWriteStream"
            | "ChildProcess"
    );
    (base, if is_emitter { EMITTER } else { &[] })
}

/// Dispatch a method call on a native stdlib instance (`recv` carries a
/// `@@native` tag). Called from `host::call_method` before the generic object
/// method resolution.
pub fn instance_call(
    tag: &str,
    recv: &Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match tag {
        "Buffer" => buffer::instance_call(recv, method, &args),
        "Timeout" | "Immediate" => timers::instance_call(recv, method, &args),
        "IntervalIterator" => timers::interval_call(recv, method, &args),
        "Date" => date::instance_call(recv, method, &args),
        "StringDecoder" => string_decoder::instance_call(recv, method, &args),
        "WeakRef" => typedarray::weakref_call(recv, method),
        "FinalizationRegistry" => typedarray::finalization_registry_call(recv, method, &args),
        "TextEncoder" => typedarray::text_encoder_call(recv, method, &args),
        "TextDecoder" => typedarray::text_decoder_call(recv, method, &args),
        "TypedArray" => typedarray::instance_call(recv, method, &args),
        t if fetch::is_class(t) => fetch::instance_call(t, recv, method, &args),
        "Hash" => crypto::instance_call(recv, method, &args),
        "Hmac" => crypto::hmac_instance_call(recv, method, &args),
        "Interface" => readline::instance_call(recv, method, args),
        "Script" => vm::instance_call(recv, method, args),
        "URLSearchParams" => url::search_params_call(recv, method, &args),
        "UdpSocket" => dgram::instance_call(recv, method, args),
        "Worker" | "MessagePort" | "BroadcastChannel" => {
            worker_threads::instance_call(tag, recv, method, args)
        }
        "TLSServer" | "TLSSocket" => tls::instance_call(tag, recv, method, args),
        "HTTPSServerResponse" | "HTTPSClientRequest" => {
            https::instance_call(tag, recv, method, args)
        }
        "REPLServer" => repl::instance_call(recv, method, args),
        "ClusterWorker" => cluster::instance_call(recv, method, args),
        "Domain" => domain::instance_call(recv, method, args),
        "Tracing" => trace_events::instance_call(recv, method, args),
        "Http2Server" | "Http2Stream" | "Http2Session" => {
            http2::instance_call(tag, recv, method, args)
        }
        "EventEmitter" => events::instance_call(recv, method, args),
        "URL" => url::instance_call(recv, method, &args),
        "Stats" => fs::stats_call(recv, method),
        "Dirent" => fs::dirent_call(recv, method),
        "Dir" => fs::dir_call(recv, method, args),
        "FSReadStream" => fs::read_stream_call(recv, method, args),
        "FSWriteStream" => fs::write_stream_call(recv, method, args),
        "Server" | "Socket" | "BlockList" => net::instance_call(tag, recv, method, args),
        "IncomingMessage" | "ServerResponse" | "ClientRequest" | "Agent" => {
            http::instance_call(tag, recv, method, args)
        }
        "Readable" | "Writable" | "Duplex" | "Transform" | "PassThrough" | "Stream" => {
            stream::instance_call(tag, recv, method, args)
        }
        "Cipheriv" | "Decipheriv" => crypto::cipher_instance_call(tag, recv, method, &args),
        "Sign" | "Verify" => crypto::sign_verify_instance_call(tag, recv, method, &args),
        "KeyObject" => crypto::key_object_instance_call(recv, method, &args),
        "DiffieHellman" => crypto::dh_instance_call(recv, method, &args),
        "ECDH" => crypto::ecdh_instance_call(recv, method, &args),
        "X509Certificate" => crypto::x509_instance_call(recv, method, &args),
        "MIMEType" => util::mime_type_instance_call(recv, method, &args),
        "MIMEParams" => util::mime_params_instance_call(recv, method, &args),
        "Blob" | "File" => buffer::blob_call(recv, method, &args),
        "ReadStream" => tty::instance_call(recv, method, &args),
        "Resolver" => dns::resolver_instance_call(recv, method, args),
        "Histogram" => perf_hooks::histogram_instance_call(recv, method, &args),
        "PerformanceObserver" => perf_hooks::observer_instance_call(recv, method, &args),
        "PerformanceObserverEntryList" => perf_hooks::entry_list_instance_call(recv, method, &args),
        "TracingChannel" => diagnostics_channel::tracing_instance_call(recv, method, &args),
        "Serializer" | "Deserializer" => v8::instance_call(tag, recv, method, args),
        "Console" => console::instance_call(recv, method, args),
        "ChildProcess" => child_process::instance_call(recv, method, args),
        t if stream_web::is_class(t) => stream_web::instance_call(t, recv, method, args),
        "AsyncLocalStorage" | "AsyncHook" | "AsyncResource" => {
            async_hooks::instance_call(tag, recv, method, args)
        }
        "Channel" => diagnostics_channel::instance_call(recv, method, &args),
        "WriteStream" => process::stream_instance_call(recv, method, &args),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

// ── shared helpers ──────────────────────────────────────────────────────────

/// The `Received …` tail Node appends to an `ERR_INVALID_ARG_TYPE` message
/// (`internal/errors.js` `determineSpecificType`): `null`/`undefined` verbatim,
/// a primitive as `type <typeof> (<inspected>)`, an object as
/// `an instance of <Ctor>`.
pub(crate) fn received_desc(v: &Value) -> String {
    with_host(|h| {
        if matches!(v, Value::Undef) {
            return "undefined".to_string();
        }
        if h.is_null(v) {
            return "null".to_string();
        }
        let ty = h.type_of(v);
        if ty == "object" || ty == "function" {
            // `ctor_name` is empty for the builtin shapes (they carry no user
            // class), so fall back to the intrinsic constructor name.
            let name = match h.ctor_name(v) {
                n if !n.is_empty() => n,
                _ => match h.get(v) {
                    Some(JsObj::Array(_)) => "Array".into(),
                    Some(JsObj::Map { .. }) => "Map".into(),
                    Some(JsObj::Set { .. }) => "Set".into(),
                    Some(JsObj::Promise { .. }) => "Promise".into(),
                    Some(JsObj::RegExp(_)) => "RegExp".into(),
                    Some(JsObj::Object(p)) => match p.get("@@native") {
                        Some(t) => h.str_of(t),
                        None => "Object".into(),
                    },
                    _ => "Object".into(),
                },
            };
            return format!("an instance of {name}");
        }
        let shown = match ty {
            "string" => format!("'{}'", h.str_of(v)),
            "bigint" => format!("{}n", h.str_of(v)),
            "number" if matches!(v, Value::Float(f) if *f == 0.0 && f.is_sign_negative()) => {
                "-0".to_string()
            }
            _ => h.str_of(v),
        };
        format!("type {ty} ({shown})")
    })
}

/// ToString of `args[i]` (empty string if absent).
pub(crate) fn arg_str(args: &[Value], i: usize) -> String {
    with_host(|h| args.get(i).map(|v| h.str_of(v)).unwrap_or_default())
}

/// ToNumber of `args[i]` (`NaN` if absent).
pub(crate) fn arg_num(args: &[Value], i: usize) -> f64 {
    with_host(|h| args.get(i).map(|v| h.to_number(v)).unwrap_or(f64::NAN))
}

/// Lowercase hex encoding of `bytes`.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Decode a hex string to bytes (ignoring a trailing odd nibble, like Node).
pub(crate) fn from_hex(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s
        .bytes()
        .filter_map(|c| (c as char).to_digit(16).map(|d| d as u8))
        .collect();
    digits
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| (c[0] << 4) | c[1])
        .collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encoding (with `=` padding) of `bytes`.
pub(crate) fn to_base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// URL-safe base64 (RFC 4648 §5) of `bytes`: `+/` become `-_` and the `=`
/// padding is dropped. This is `buf.toString('base64url')`, which is a distinct
/// encoding from `'base64'` — not an alias. Node emits `Buffer.from([251,255,
/// 190,1]).toString('base64url')` as `-_--AQ` where `'base64'` gives `+/++AQ==`.
pub(crate) fn to_base64url(bytes: &[u8]) -> String {
    to_base64(bytes)
        .chars()
        .filter(|c| *c != '=')
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect()
}

/// Decode a base64 string to bytes (ignores whitespace and padding).
///
/// BOTH alphabets are accepted, in either direction: node decodes `-_` under
/// `'base64'` and `+/` under `'base64url'` (measured — `Buffer.from('-_-_',
/// 'base64').toString('hex')` and `Buffer.from('+/+/','base64url')
/// .toString('hex')` are both `fbffbf` on v26.7.0), so the decoder does not need
/// to know which name it was reached by. Refusing the URL-safe characters here
/// silently produced an EMPTY buffer, because an unrecognized character is
/// dropped rather than rejected.
pub(crate) fn from_base64(s: &str) -> Vec<u8> {
    let rev = |c: u8| -> Option<u32> {
        let c = match c {
            b'-' => b'+',
            b'_' => b'/',
            c => c,
        };
        B64.iter().position(|&x| x == c).map(|p| p as u32)
    };
    let vals: Vec<u32> = s.bytes().filter_map(rev).collect();
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = (chunk[0] << 18)
            | (chunk[1] << 12)
            | (chunk.get(2).copied().unwrap_or(0) << 6)
            | chunk.get(3).copied().unwrap_or(0);
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
