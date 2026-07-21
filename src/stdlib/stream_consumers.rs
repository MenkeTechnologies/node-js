//! Node `stream/consumers` module: read an entire stream to a single value.
//!
//! Each function (`text`/`json`/`arrayBuffer`/`buffer`/`bytes`/`blob`) returns a
//! Promise that settles once the stream ends. The reader is a small compiled JS
//! factory (same re-entrant `compile_completion` → `load_merged` → `run_chunk_on`
//! path `util.promisify` uses): it attaches `data`/`end`/`error` listeners on the
//! stream's real EventEmitter surface, accumulates chunks with `Buffer.concat`,
//! and resolves the Promise on `end` (rejecting on `error`). Building it in JS
//! means the genuine Promise + emitter machinery does the work — no bespoke
//! native listener/accumulator.
//!
//! Runtime byte container: this engine represents raw bytes as a `Buffer`, so
//! `arrayBuffer`/`bytes` resolve with a `Buffer` (matching `buffer::blob_call`,
//! which likewise resolves its `arrayBuffer`/`bytes` accessors with a `Buffer`
//! rather than a bare `ArrayBuffer`/`Uint8Array`). `blob` resolves a real `Blob`.

use crate::host::with_host;
use fusevm::Value;

/// The free functions exported by `require('stream/consumers')`.
pub const METHODS: &[&str] = &["text", "json", "arrayBuffer", "buffer", "bytes", "blob"];

/// The compiled reader factory: `(stream, kind) => Promise<value>`. A `data`
/// listener collects chunks (string chunks are wrapped to `Buffer`); the `end`
/// listener concatenates and finalizes per `kind`; `error` rejects.
const CONSUMER_SRC: &str = "(function (stream, kind) {\n\
  var B = require('buffer');\n\
  return new Promise(function (resolve, reject) {\n\
    var chunks = [];\n\
    stream.on('data', function (c) {\n\
      chunks.push(typeof c === 'string' ? B.Buffer.from(c) : c);\n\
    });\n\
    stream.on('error', function (e) { reject(e); });\n\
    stream.on('end', function () {\n\
      try {\n\
        var buf = B.Buffer.concat(chunks);\n\
        if (kind === 'text') return resolve(buf.toString('utf8'));\n\
        if (kind === 'json') return resolve(JSON.parse(buf.toString('utf8')));\n\
        if (kind === 'blob') return resolve(new B.Blob([buf]));\n\
        return resolve(buf);\n\
      } catch (err) { reject(err); }\n\
    });\n\
  });\n\
})";

/// Compile a single JS expression and run it on the LIVE host, returning its
/// completion value (mirrors `util`'s promisify factory path).
fn run_completion(src: &str) -> Result<Value, String> {
    let prog = crate::compile_completion(src)?;
    let chunk = crate::load_merged(prog);
    crate::host::run_chunk_on(chunk)
}

/// Module free-function dispatch (`consumers.text`, `consumers.json`, …).
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let kind = match method {
        "text" | "json" | "arrayBuffer" | "buffer" | "bytes" | "blob" => method,
        _ => return None,
    };
    let stream = args.first().cloned().unwrap_or(Value::Undef);
    Some(consume(stream, kind))
}

/// Invoke the reader factory with `stream` and the `kind` selector, returning the
/// Promise it produces.
fn consume(stream: Value, kind: &str) -> Result<Value, String> {
    let factory = run_completion(CONSUMER_SRC)?;
    let kv = with_host(|h| h.new_str(kind.to_string()));
    crate::host::invoke(&factory, vec![stream, kv], None)
}
