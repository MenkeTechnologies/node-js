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
pub mod buffer;
pub mod crypto;
pub mod date;
pub mod string_decoder;
pub mod typedarray;
pub mod events;
pub mod fs;
pub mod http;
pub mod net;
pub mod os;
pub mod path;
pub mod process;
pub mod querystring;
pub mod stream;
pub mod tty;
pub mod url;
pub mod util;
pub mod zlib;
pub mod async_hooks;
pub mod child_process;
pub mod console;
pub mod diagnostics_channel;
pub mod dns;
pub mod perf_hooks;
pub mod punycode;
pub mod timers;
pub mod util_types;
pub mod v8;
pub mod fs_promises;
pub mod readline;
pub mod vm;
pub mod dgram;
pub mod https;
pub mod repl;
pub mod tls;
pub mod worker_threads;
pub mod cluster;
pub mod domain;
pub mod http2;
pub mod trace_events;
pub mod node_module;
pub mod stream_consumers;
pub mod stream_promises;
pub mod stream_web;

/// Native-heavy core modules that node-js does not yet implement (TLS handshakes,
/// HTTP/2 framing, OS worker threads sharing the thread-local heap, UDP sockets,
/// V8 inspector, etc.). `require`ing them succeeds and yields a namespace so that
/// programs which import-then-conditionally-use them still load; ACTUALLY calling
/// a method throws `Error: <mod>.<method> is not implemented in node-js`. This is
/// an honest not-yet-built surface, never a silent fake.
pub const UNIMPLEMENTED_MODULES: &[&str] = &[
    "inspector", "wasi",
];

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
        // `path/posix` is exactly our POSIX `path`; `assert/strict` is `assert`
        // (our assert is already strict-equality based).
        "path/posix" => Some("path"),
        "path/win32" => Some("path"),
        // `sys` is the long-deprecated alias for `util`.
        "sys" => Some("util"),
        "assert/strict" => Some("assert"),
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
    match ns {
        "fs" => fs::METHODS.contains(&m),
        "path" => path::METHODS.contains(&m),
        "os" => os::METHODS.contains(&m),
        "util" => util::METHODS.contains(&m),
        "assert" => assert::METHODS.contains(&m),
        "assertStrict" => assert::METHODS.contains(&m),
        "crypto" => crypto::METHODS.contains(&m),
        "Buffer" => buffer::STATIC_METHODS.contains(&m),
        "buffer" => m == "Buffer" || buffer::MODULE_METHODS.contains(&m),
        "Date" => date::STATIC_METHODS.contains(&m),
        "TextEncoder" | "TextDecoder" => false,
        n if typedarray::is_ctor(n) => typedarray::STATIC_METHODS.contains(&m),
        "url" => url::MODULE_METHODS.contains(&m) || m == "URL",
        "net" => net::MODULE_METHODS.contains(&m),
        "http" => http::MODULE_METHODS.contains(&m),
        "stream" => stream::METHODS.contains(&m),
        "worker_threads" => worker_threads::METHODS.contains(&m),
        "zlib" => zlib::MODULE_METHODS.contains(&m),
        "querystring" => querystring::METHODS.contains(&m),
        "tty" => tty::METHODS.contains(&m),
        "process" => process::METHODS.contains(&m),
        "EventEmitter" => m == "EventEmitter" || events::STATIC_METHODS.contains(&m),
        "console" => console::METHODS.contains(&m),
        "child_process" => child_process::METHODS.contains(&m),
        "dns" => dns::METHODS.contains(&m),
        "punycode" => punycode::METHODS.contains(&m),
        "timers" => timers::METHODS.contains(&m),
        "timers/promises" => timers::PROMISES_METHODS.contains(&m),
        "perf_hooks" | "performance" => perf_hooks::METHODS.contains(&m),
        "async_hooks" => async_hooks::METHODS.contains(&m),
        "util/types" => util_types::METHODS.contains(&m),
        "diagnostics_channel" => diagnostics_channel::METHODS.contains(&m),
        "v8" => v8::METHODS.contains(&m),
        "readline" => readline::METHODS.contains(&m),
        "vm" => vm::METHODS.contains(&m),
        "fs/promises" => fs_promises::METHODS.contains(&m),
        "dgram" => dgram::MODULE_METHODS.contains(&m),
        "dns/promises" => matches!(
            m,
            "lookup" | "lookupService" | "resolve" | "resolve4" | "resolve6" | "resolveMx"
                | "resolveTxt" | "resolveCname" | "resolveNs" | "resolvePtr" | "resolveSrv"
                | "resolveSoa" | "resolveNaptr" | "resolveCaa" | "resolveTlsa" | "resolveAny"
                | "reverse" | "getServers" | "setServers" | "getDefaultResultOrder"
                | "setDefaultResultOrder"
        ),
        "tls" => tls::MODULE_METHODS.contains(&m),
        "https" => https::MODULE_METHODS.contains(&m),
        "repl" => repl::METHODS.contains(&m),
        "cluster" => cluster::METHODS.contains(&m),
        "domain" => domain::METHODS.contains(&m),
        "http2" => http2::METHODS.contains(&m),
        "trace_events" => trace_events::METHODS.contains(&m),
        "module" => node_module::METHODS.contains(&m),
        "Module" => node_module::MODULE_STATIC_METHODS.contains(&m),
        "stream/consumers" => stream_consumers::METHODS.contains(&m),
        "stream/promises" => stream_promises::is_method(m),
        // Any method on an unimplemented namespace routes to `call`, which throws
        // an honest "not implemented" error (so `mod.foo()` fails clearly rather
        // than silently returning undefined).
        _ if is_unimplemented(ns) => true,
        _ => false,
    }
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
        "path" => path::call(m, args)?,
        "os" => os::call(m, args)?,
        "util" => util::call(m, args)?,
        "assert" => assert::call(m, args)?,
        "assertStrict" => assert::strict_call(m, args)?,
        "crypto" => crypto::call(m, args)?,
        "Buffer" => buffer::static_call(m, args)?,
        "buffer" if m == "Buffer" => Ok(with_host(|h| h.alloc(JsObj::Builtin("Buffer".into())))),
        "buffer" => buffer::module_call(m, args)?,
        "Date" => date::static_call(m, args)?,
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
        "EventEmitter" if m == "EventEmitter" => {
            Ok(with_host(|h| h.alloc(JsObj::Builtin("EventEmitter".into()))))
        }
        "EventEmitter" => events::static_call(m, args)?,
        "console" => console::call(m, args)?,
        "child_process" => child_process::call(m, args)?,
        "dns" => dns::call(m, args)?,
        "punycode" => punycode::call(m, args)?,
        "timers" => timers::call(m, args)?,
        "timers/promises" => timers::promises_call(m, args)?,
        "perf_hooks" | "performance" => perf_hooks::call(m, args)?,
        "async_hooks" => async_hooks::call(m, args)?,
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
        _ if is_unimplemented(ns) => {
            Err(format!("Error: {ns}.{m} is not implemented in node-js"))
        }
        _ => return None,
    })
}

/// A non-function constant on a stdlib namespace (`path.sep`, `os.EOL`,
/// `buffer.Buffer`, `url.URL`), reachable via `namespace_property`.
pub fn constant(ns: &str, name: &str) -> Option<Value> {
    match ns {
        // `path.posix` is our POSIX path itself; expose it (and `path.win32` as a
        // best-effort alias) as a nested namespace so `path.posix.join(...)` works.
        "path" if name == "posix" || name == "win32" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("path".into()))))
        }
        "path" => path::constant(name),
        "os" => os::constant(name),
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
        "assert" if name == "AssertionError" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("AssertionError".into()))))
        }
        "assert" if name == "strict" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("assertStrict".into()))))
        }
        "stream" => stream::constant(name),
        "http" => http::constant(name),
        "string_decoder" if name == "StringDecoder" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("StringDecoder".into()))))
        }
        "process" => process::constant(name),
        "EventEmitter" if name == "EventEmitter" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("EventEmitter".into()))))
        }
        "perf_hooks" | "performance" => perf_hooks::constant(name),
        "dns" => dns::constant(name),
        "punycode" => punycode::constant(name),
        "async_hooks" if matches!(name, "AsyncLocalStorage" | "AsyncResource") => {
            Some(with_host(|h| h.alloc(JsObj::Builtin(name.into()))))
        }
        "vm" if name == "Script" => Some(with_host(|h| h.alloc(JsObj::Builtin("Script".into())))),
        "url" if name == "URLSearchParams" => {
            Some(with_host(|h| h.alloc(JsObj::Builtin("URLSearchParams".into()))))
        }
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
                "Sign" | "Verify" | "KeyObject" | "DiffieHellman" | "ECDH" | "X509Certificate"
                    | "Hash" | "Hmac" | "Cipheriv" | "Decipheriv"
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
        "Buffer" => Some(buffer::static_call("from", args).unwrap_or(Ok(Value::Undef))),
        "Date" => Some(date::construct(args)),
        "StringDecoder" => Some(string_decoder::construct(args)),
        "WeakRef" => Some(typedarray::construct_weakref(args)),
        "TextEncoder" => Some(typedarray::construct_text_encoder()),
        "TextDecoder" => Some(typedarray::construct_text_decoder(args)),
        n if typedarray::is_ctor(n) => Some(typedarray::construct(n, args)),
        n if stream::is_class(n) => Some(Ok(stream::construct(n))),
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
        "Socket" | "Stream" | "Server" | "SocketAddress" | "BlockList" => net::construct(name, args),
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

/// Whether `name` is a method of a native instance tagged `tag`. Used by
/// `get_property` so a method *read* (`server.listen.apply(...)`, the express
/// listen path) yields a bound method rather than `undefined` — the method is
/// still dispatched through `instance_call` when the bound method is invoked.
pub fn instance_has_method(tag: &str, name: &str) -> bool {
    // Shared EventEmitter surface for the emitter-backed instances.
    const EMITTER: &[&str] = &[
        "on", "once", "emit", "addListener", "prependListener", "prependOnceListener",
        "removeListener", "off", "removeAllListeners", "listeners", "listenerCount",
        "eventNames", "setMaxListeners", "getMaxListeners",
    ];
    let base: &[&str] = match tag {
        "Server" => &["listen", "close", "address"],
        "Socket" => &[
            "write", "end", "destroy", "pause", "resume", "setEncoding", "setKeepAlive",
            "setNoDelay", "setTimeout", "ref", "unref", "connect",
        ],
        "ServerResponse" => &[
            "writeHead", "setHeader", "getHeader", "getHeaderNames", "getHeaders", "hasHeader",
            "removeHeader", "write", "end", "flushHeaders",
        ],
        "IncomingMessage" => &["pause", "resume", "setEncoding", "destroy"],
        "Buffer" => &[
            "toString", "toJSON", "equals", "slice", "subarray", "readUInt8", "includes",
            "indexOf", "lastIndexOf", "write", "copy", "fill", "compare", "readUInt16BE",
            "readUInt16LE", "writeUInt8", "writeUInt16BE", "writeUInt16LE",
        ],
        "Readable" | "Writable" | "Duplex" | "Transform" | "PassThrough" | "Stream" => {
            &["read", "write", "end", "pipe", "pause", "resume", "setEncoding", "destroy", "push"]
        }
        "URL" => &["toString", "toJSON"],
        "AsyncLocalStorage" => async_hooks::ALS_METHODS,
        "AsyncHook" => async_hooks::HOOK_METHODS,
        "Channel" => &["subscribe", "unsubscribe", "publish"],
        "WriteStream" => &["write", "end", "on", "once", "removeListener", "cork", "uncork", "setEncoding"],
        "Hmac" => &["update", "digest"],
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
            "generateKeys", "computeSecret", "getPrime", "getGenerator", "getPublicKey",
            "getPrivateKey", "setPublicKey", "setPrivateKey",
        ],
        "ECDH" => &["generateKeys", "computeSecret", "getPublicKey", "getPrivateKey", "setPrivateKey"],
        "X509Certificate" => &["toString"],
        "MIMEType" => util::MIME_TYPE_METHODS,
        "MIMEParams" => util::MIME_PARAMS_METHODS,
        t if stream_web::is_class(t) => stream_web::methods_for(t),
        _ => &[],
    };
    let is_emitter = matches!(
        tag,
        "Server" | "Socket" | "ServerResponse" | "IncomingMessage" | "EventEmitter" | "Readable"
            | "Writable" | "Duplex" | "Transform" | "PassThrough" | "Stream" | "UdpSocket"
            | "Worker" | "MessagePort" | "TLSServer" | "TLSSocket" | "HTTPSServerResponse"
            | "HTTPSClientRequest" | "ClusterWorker" | "Domain" | "Http2Server" | "Http2Stream"
            | "Http2Session" | "ClientRequest" | "FSReadStream" | "FSWriteStream" | "ChildProcess"
    );
    base.contains(&name) || (is_emitter && EMITTER.contains(&name))
}

/// Dispatch a method call on a native stdlib instance (`recv` carries a
/// `@@native` tag). Called from `host::call_method` before the generic object
/// method resolution.
pub fn instance_call(tag: &str, recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match tag {
        "Buffer" => buffer::instance_call(recv, method, &args),
        "Date" => date::instance_call(recv, method, &args),
        "StringDecoder" => string_decoder::instance_call(recv, method, &args),
        "WeakRef" => typedarray::weakref_call(recv, method),
        "TextEncoder" => typedarray::text_encoder_call(recv, method, &args),
        "TextDecoder" => typedarray::text_decoder_call(recv, method, &args),
        "TypedArray" => typedarray::instance_call(recv, method, &args),
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
        "HTTPSServerResponse" | "HTTPSClientRequest" => https::instance_call(tag, recv, method, args),
        "REPLServer" => repl::instance_call(recv, method, args),
        "ClusterWorker" => cluster::instance_call(recv, method, args),
        "Domain" => domain::instance_call(recv, method, args),
        "Tracing" => trace_events::instance_call(recv, method, args),
        "Http2Server" | "Http2Stream" | "Http2Session" => http2::instance_call(tag, recv, method, args),
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
        "AsyncLocalStorage" | "AsyncHook" => async_hooks::instance_call(tag, recv, method, args),
        "Channel" => diagnostics_channel::instance_call(recv, method, &args),
        "WriteStream" => process::stream_instance_call(recv, method, &args),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
    }
}

// ── shared helpers ──────────────────────────────────────────────────────────

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
    let digits: Vec<u8> = s.bytes().filter_map(|c| (c as char).to_digit(16).map(|d| d as u8)).collect();
    digits.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] << 4) | c[1]).collect()
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
        out.push(if chunk.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Decode a standard base64 string to bytes (ignores whitespace and padding).
pub(crate) fn from_base64(s: &str) -> Vec<u8> {
    let rev = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
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
