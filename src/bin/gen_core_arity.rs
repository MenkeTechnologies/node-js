//! Regenerate `src/core_arity.rs` — the `name`/`length` of every function a
//! core module exports (`fs.readFileSync`, `util.format`, `path.join`, …),
//! keyed the way this frontend spells it: `<namespace>.<member>`.
//!
//! ```text
//! cargo run --bin gen-core-arity > src/core_arity.rs
//! ```
//!
//! These arities are specified nowhere: they are whatever node's own JavaScript
//! declares (`util.format.length` is 0, `fs.readFileSync.length` is 2), and they
//! change between releases. That is the reason to MEASURE them from the
//! reference binary rather than write them down. Kept apart from
//! `src/arity.rs` (`gen-arity`, the ECMAScript intrinsics) so each table is
//! regenerated whole from its own question.
//!
//! Dev tool — needs a reference `node`, so nothing in CI runs it; the generated
//! table is checked in.

use std::io::Write;
use std::process::Command;

/// One `key\tname\tlength` row per plain function a core module owns. A class
/// export (`fs.Stats`) is a constructor this frontend keys by its own name, so
/// only lower-case members are recorded. `spellings` is the namespace name the
/// frontend gives each module: `require('events')` is the `EventEmitter`
/// constructor and `require('stream')` the `Stream` one.
const ENUMERATE: &str = r#"
const spellings = {
  assert: 'assert', buffer: 'buffer', child_process: 'child_process', console: 'console',
  crypto: 'crypto', dns: 'dns', 'dns/promises': 'dns/promises', events: 'EventEmitter',
  fs: 'fs', 'fs/promises': 'fs/promises', http: 'http', https: 'https', net: 'net',
  os: 'os', path: 'path', 'path/win32': 'path/win32', perf_hooks: 'perf_hooks',
  process: 'process', querystring: 'querystring', readline: 'readline',
  'readline/promises': 'readline/promises', stream: 'Stream', 'stream/promises': 'stream/promises',
  'stream/consumers': 'stream/consumers', string_decoder: 'string_decoder', timers: 'timers',
  'timers/promises': 'timers/promises', tty: 'tty', url: 'url', util: 'util',
  'util/types': 'util/types', v8: 'v8', vm: 'vm', zlib: 'zlib', worker_threads: 'worker_threads',
  async_hooks: 'async_hooks', diagnostics_channel: 'diagnostics_channel', dgram: 'dgram',
  cluster: 'cluster', tls: 'tls', http2: 'http2', domain: 'domain', trace_events: 'trace_events',
  repl: 'repl',
};
const rows = [];
const symbolRows = [];
for (const [id, ns] of Object.entries(spellings)) {
  let mod; try { mod = require(id); } catch { continue; }
  for (const k of Object.getOwnPropertyNames(mod)) {
    let d; try { d = Object.getOwnPropertyDescriptor(mod, k); } catch { continue; }
    if (!d || /^[A-Z]/.test(k)) continue;
    // A lazily loaded export (`crypto.getRandomValues`, `util.aborted`) is an
    // ACCESSOR whose getter yields the function, so it is read through.
    let f = d.value;
    if (d.get) { try { f = mod[k]; } catch { continue; } }
    if (typeof f !== 'function') continue;
    // The fourth column: whether the function owns a `prototype` (an ordinary
    // `function` does; an arrow, a method or a bound function does not).
    rows.push(ns + '.' + k + '\t' + f.name + '\t' + f.length + '\t' + (Object.hasOwn(f, 'prototype') ? 1 : 0));
    // The function's own SYMBOL-keyed properties (`os.freemem[Symbol.toPrimitive]`,
    // `setTimeout[util.promisify.custom]`): the key as this frontend spells a
    // symbol (`@@<name>` for a well-known one, `for:<key>` for a registered
    // one), whether it is an accessor, and its attributes.
    for (const s of Object.getOwnPropertySymbols(f)) {
      const sd = Object.getOwnPropertyDescriptor(f, s);
      const desc = s.description;
      const spelled = Symbol.keyFor(s) !== undefined ? 'for:' + Symbol.keyFor(s) :
        desc.startsWith('Symbol.') ? '@@' + desc.slice(7) : null;
      if (spelled === null) continue;
      symbolRows.push([ns + '.' + k, spelled, sd.get ? 1 : 0, sd.enumerable ? 1 : 0,
        sd.writable ? 1 : 0, sd.configurable ? 1 : 0].join('\t'));
    }
  }
}
console.log(rows.join('\n'));
// Each module's own ENUMERABLE keys in node's order (`Object.keys`), values and
// classes included — the order and membership `Object.keys(require(id))` shows.
console.log('===KEYS===');
for (const [id, ns] of Object.entries(spellings)) {
  let mod; try { mod = require(id); } catch { continue; }
  console.log(ns + '\t' + Object.keys(mod).join(','));
}
// The web and node classes this frontend models as native prototypes, with
// each prototype's own STRING keys in node's order (`getOwnPropertyNames`).
console.log('===PROTOS===');
const protoClasses = [
  'ReadableStream', 'ReadableStreamDefaultReader', 'ReadableStreamBYOBReader',
  'ReadableStreamDefaultController', 'ReadableByteStreamController', 'ReadableStreamBYOBRequest',
  'WritableStream', 'WritableStreamDefaultWriter', 'WritableStreamDefaultController',
  'TransformStream', 'TransformStreamDefaultController', 'ByteLengthQueuingStrategy',
  'CountQueuingStrategy', 'TextEncoderStream', 'TextDecoderStream', 'CompressionStream',
  'DecompressionStream', 'Performance', 'Crypto', 'CryptoKey', 'SubtleCrypto',
  'EventTarget', 'Event', 'CustomEvent', 'AbortController', 'AbortSignal',
];
for (const name of protoClasses) {
  const C = globalThis[name];
  if (typeof C === 'function' && C.prototype) console.log(name + '\t' + Object.getOwnPropertyNames(C.prototype).join(','));
}
// The `name` and `length` of those prototypes' METHODS (data members that are
// functions), keyed `@proto:<Class>:<method>` as this frontend spells them.
console.log('===PROTO_ARITY===');
for (const name of protoClasses) {
  const C = globalThis[name];
  if (typeof C !== 'function' || !C.prototype) continue;
  for (const k of Object.getOwnPropertyNames(C.prototype)) {
    if (k === 'constructor') continue;
    const d = Object.getOwnPropertyDescriptor(C.prototype, k);
    if (d && typeof d.value === 'function') console.log('@proto:' + name + ':' + k + '\t' + d.value.name + '\t' + d.value.length);
  }
}
console.log('===SYMBOLS===');
console.log(symbolRows.join('\n'));
// EventEmitter.prototype's methods (`name`/`length`, an alias reporting the
// method it IS: `on` is `addListener`), then, for each emitter-backed class
// this frontend models (keyed by its native tag), the EventEmitter methods its
// prototype chain OVERRIDES rather than inherits: a Readable's `on` is
// `Readable.prototype.on`, a ChildProcess's is `EventEmitter.prototype.on`.
console.log('===EMITTER===');
{
  const EE = require('events');
  const P = EE.prototype;
  const methods = Object.getOwnPropertyNames(P).filter((k) => k !== 'constructor' && typeof P[k] === 'function');
  for (const k of methods) console.log('@proto:EventEmitter:' + k + '\t' + P[k].name + '\t' + P[k].length);
  console.log('===EMITTER_OVERRIDES===');
  const s = require('stream'), net = require('net'), http = require('http'), https = require('https');
  const fs = require('fs'), tls = require('tls'), dgram = require('dgram'), http2 = require('http2');
  const wt = require('worker_threads'), cluster = require('cluster'), domain = require('domain');
  const classes = {
    EventEmitter: EE, Stream: s.Stream, Readable: s.Readable, Writable: s.Writable,
    Duplex: s.Duplex, Transform: s.Transform, PassThrough: s.PassThrough,
    Socket: net.Socket, Server: net.Server, ChildPipe: net.Socket,
    IncomingMessage: http.IncomingMessage, ServerResponse: http.ServerResponse,
    ClientRequest: http.ClientRequest, HTTPSServerResponse: http.ServerResponse,
    HTTPSClientRequest: http.ClientRequest, FSReadStream: fs.ReadStream,
    FSWriteStream: fs.WriteStream, TLSSocket: tls.TLSSocket, TLSServer: tls.Server,
    UdpSocket: dgram.Socket, MessagePort: wt.MessagePort, Worker: wt.Worker,
    ClusterWorker: cluster.Worker, Domain: domain.Domain,
    ChildProcess: require('child_process').ChildProcess,
    Http2Server: http2.Http2Server, Http2Session: http2.Http2Session, Http2Stream: http2.Http2Stream,
  };
  for (const [tag, K] of Object.entries(classes)) {
    if (typeof K !== 'function' || !K.prototype) continue;
    console.log(tag + '\t' + methods.filter((m) => K.prototype[m] !== P[m]).join(','));
  }
}
// Each module's ACCESSOR exports (`timers.promises`, `fs.ReadStream`,
// `crypto.webcrypto`): whether a setter exists, the attributes, and the
// getter's and setter's `name`.
console.log('===ACCESSORS===');
for (const [id, ns] of Object.entries(spellings)) {
  let mod; try { mod = require(id); } catch { continue; }
  for (const k of Object.getOwnPropertyNames(mod)) {
    let d; try { d = Object.getOwnPropertyDescriptor(mod, k); } catch { continue; }
    if (!d || !(d.get || d.set)) continue;
    console.log([ns, k, d.set ? 1 : 0, d.enumerable ? 1 : 0, d.configurable ? 1 : 0,
      d.get ? d.get.name : '', d.set ? d.set.name : ''].join('\t'));
  }
}
"#;

fn main() {
    let oracle = std::env::var("NODE_JS_PARITY_NODE").unwrap_or_else(|_| "node".into());
    let dir = std::env::temp_dir().join(format!("node-js-core-arity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script = dir.join("enumerate.js");
    std::fs::write(&script, ENUMERATE).expect("write script");
    let out = Command::new(&oracle)
        .arg(&script)
        .output()
        .unwrap_or_else(|e| panic!("running reference `{oracle}`: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        panic!(
            "reference `{oracle}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let version = Command::new(&oracle)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (rows_text, rest) = text
        .split_once("===KEYS===\n")
        .expect("enumerator emitted no key section");
    let (keys_text, rest) = rest
        .split_once("===PROTOS===\n")
        .expect("enumerator emitted no prototype section");
    let (protos_text, rest) = rest
        .split_once("===PROTO_ARITY===\n")
        .expect("enumerator emitted no prototype-arity section");
    let (proto_arity_text, rest) = rest
        .split_once("===SYMBOLS===\n")
        .expect("enumerator emitted no symbol-property section");
    let (symbols_text, rest) = rest
        .split_once("===EMITTER===\n")
        .expect("enumerator emitted no emitter section");
    let (emitter_text, rest) = rest
        .split_once("===EMITTER_OVERRIDES===\n")
        .expect("enumerator emitted no emitter-override section");
    let (overrides_text, accessors_text) = rest
        .split_once("===ACCESSORS===\n")
        .expect("enumerator emitted no accessor-export section");
    let mut rows: Vec<(String, String, String, bool)> = rows_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            (
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
                it.next() == Some("1"),
            )
        })
        .collect();
    // Sorted for binary search and for a reviewable regeneration diff.
    rows.sort();
    rows.dedup();
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    writeln!(w, "//! `name` and `length` of the core-module functions.").unwrap();
    writeln!(w, "//!").unwrap();
    writeln!(
        w,
        "//! GENERATED by `cargo run --bin gen-core-arity` from {version} — do not edit by hand."
    )
    .unwrap();
    writeln!(
        w,
        "//! See `src/bin/gen_core_arity.rs` for what is in the table."
    )
    .unwrap();
    writeln!(w).unwrap();
    writeln!(
        w,
        "/// `(key, name, length)` of each function a core module exports, sorted by\n\
         /// key. The key is the [`crate::host::JsObj::Builtin`] spelling,\n\
         /// `<namespace>.<member>` (`fs.readFileSync`, `EventEmitter.once`).\n\
         pub const CORE_ARITY: &[(&str, &str, u32)] = &["
    )
    .unwrap();
    for (key, name, len, _) in &rows {
        writeln!(w, "    ({key:?}, {name:?}, {len}),").unwrap();
    }
    writeln!(w, "];").unwrap();
    writeln!(
        w,
        "\n/// The keys of [`CORE_ARITY`] whose function owns a `prototype` (an ordinary\n\
         /// `function`; an arrow, a method or a bound function owns none). Sorted.\n\
         pub const CORE_PROTOTYPED: &[&str] = &["
    )
    .unwrap();
    for (key, _, _, proto) in &rows {
        if *proto {
            writeln!(w, "    {key:?},").unwrap();
        }
    }
    writeln!(w, "];").unwrap();
    let mut keys: Vec<(String, Vec<String>)> = keys_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (ns, list) = l.split_once('\t').unwrap_or((l, ""));
            let list = list
                .split(',')
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .collect();
            (ns.to_string(), list)
        })
        .collect();
    keys.sort();
    writeln!(
        w,
        "\n/// Each core module's own enumerable keys in node's order (`Object.keys`),\n\
         /// keyed by namespace spelling and sorted by it; the keys within a row are\n\
         /// not sorted.\n\
         pub const CORE_KEYS: &[(&str, &[&str])] = &["
    )
    .unwrap();
    for (ns, list) in &keys {
        let list = list
            .iter()
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(w, "    ({ns:?}, &[{list}]),").unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut protos: Vec<(String, String)> = protos_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (c, list) = l.split_once('\t').unwrap_or((l, ""));
            (c.to_string(), list.to_string())
        })
        .collect();
    protos.sort();
    writeln!(
        w,
        "\n/// The own string keys of each modelled web/node class prototype, in node's\n\
         /// order (`Object.getOwnPropertyNames(C.prototype)`), by class name; sorted\n\
         /// by class, the names within a row are not.\n\
         pub const CLASS_PROTO_ORDER: &[(&str, &[&str])] = &["
    )
    .unwrap();
    for (c, list) in &protos {
        let list = list
            .split(',')
            .filter(|k| !k.is_empty())
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(w, "    ({c:?}, &[{list}]),").unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut parity_rows: Vec<(String, String, String)> = proto_arity_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            (
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
            )
        })
        .collect();
    parity_rows.sort();
    writeln!(
        w,
        "\n/// `(key, name, length)` of each modelled class prototype's methods, keyed\n\
         /// `@proto:<Class>:<method>` by the class's NAME; sorted by key.\n\
         pub const CLASS_PROTO_ARITY: &[(&str, &str, u32)] = &["
    )
    .unwrap();
    for (key, name, len) in &parity_rows {
        writeln!(w, "    ({key:?}, {name:?}, {len}),").unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut symbol_rows: Vec<Vec<String>> = symbols_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    symbol_rows.sort();
    writeln!(
        w,
        "\n/// The own symbol-keyed properties of the core-module functions:\n\
         /// `(key, symbol, accessor, enumerable, writable, configurable)`. The symbol\n\
         /// is `@@<name>` for a well-known one, `for:<key>` for one in the\n\
         /// `Symbol.for` registry; `accessor` marks a getter (`writable` is then\n\
         /// meaningless). Sorted by key.\n\
         pub const CORE_SYMBOL_PROPS: &[(&str, &str, bool, bool, bool, bool)] = &["
    )
    .unwrap();
    for r in &symbol_rows {
        let flag = |i: usize| r.get(i).map(String::as_str) == Some("1");
        writeln!(
            w,
            "    ({:?}, {:?}, {}, {}, {}, {}),",
            r[0],
            r[1],
            flag(2),
            flag(3),
            flag(4),
            flag(5)
        )
        .unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut accessor_rows: Vec<Vec<String>> = accessors_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    accessor_rows.sort();
    writeln!(
        w,
        "\n/// Each core module's ACCESSOR exports:\n\
         /// `(namespace, key, has_setter, enumerable, configurable, getter_name,\n\
         /// setter_name)`, keyed by namespace spelling. Sorted by namespace, then key.\n\
         pub const CORE_ACCESSOR_EXPORTS: &[(&str, &str, bool, bool, bool, &str, &str)] = &["
    )
    .unwrap();
    for r in &accessor_rows {
        let col = |i: usize| r.get(i).map(String::as_str).unwrap_or("");
        writeln!(
            w,
            "    ({:?}, {:?}, {}, {}, {}, {:?}, {:?}),",
            col(0),
            col(1),
            col(2) == "1",
            col(3) == "1",
            col(4) == "1",
            col(5),
            col(6)
        )
        .unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut emitter_rows: Vec<(String, String, String)> = emitter_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            (
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().to_string(),
            )
        })
        .collect();
    emitter_rows.sort();
    writeln!(
        w,
        "\n/// `(key, name, length)` of `EventEmitter.prototype`'s methods, keyed\n\
         /// `@proto:EventEmitter:<method>`; an alias reports the method it is\n\
         /// (`on` is `addListener`). Sorted by key.\n\
         pub const EMITTER_PROTO_ARITY: &[(&str, &str, u32)] = &["
    )
    .unwrap();
    for (key, name, len) in &emitter_rows {
        writeln!(w, "    ({key:?}, {name:?}, {len}),").unwrap();
    }
    writeln!(w, "];").unwrap();
    let mut override_rows: Vec<(String, String)> = overrides_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (tag, list) = l.split_once('\t').unwrap_or((l, ""));
            (tag.to_string(), list.to_string())
        })
        .collect();
    override_rows.sort();
    writeln!(
        w,
        "\n/// For each emitter-backed native class (by tag), the `EventEmitter` methods\n\
         /// its prototype chain OVERRIDES instead of inheriting from\n\
         /// `EventEmitter.prototype`. Sorted by tag; the names within a row are not.\n\
         pub const EMITTER_OVERRIDES: &[(&str, &[&str])] = &["
    )
    .unwrap();
    for (tag, list) in &override_rows {
        let list = list
            .split(',')
            .filter(|k| !k.is_empty())
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(w, "    ({tag:?}, &[{list}]),").unwrap();
    }
    writeln!(w, "];").unwrap();
}
