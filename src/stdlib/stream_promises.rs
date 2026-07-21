//! Node `stream/promises` module: the Promise-based `finished` and `pipeline`.
//!
//! `require('stream/promises')` exposes promise-returning versions of
//! `stream.finished` and `stream.pipeline`. Rather than duplicate the listener
//! bookkeeping already in `stream.rs`, each wraps the callback-based
//! `require('stream')` free function in a `Promise` — the same compile-a-JS-factory
//! technique `util.promisify` uses (`crate::compile_completion` + `load_merged` +
//! `host::run_chunk_on`, then invoke the factory).
//!
//! The returned promise is a REAL pending promise: `stream.finished(stream, cb)`
//! registers listeners and drains its callback on the first terminal event
//! (`end`/`finish`/`close`) or on `error`, at which point the callback settles the
//! promise. A stream that has ALREADY reached a terminal state settles immediately
//! (the callback-based `finished` fires synchronously in that case). No faked
//! resolution — a stream that never terminates leaves the promise pending, exactly
//! as Node does.

use fusevm::Value;

/// `stream/promises` module free-functions routed through `stdlib::call`.
pub const METHODS: &[&str] = &["finished", "pipeline"];

/// True if `name` is a `stream/promises` free function (for the parent's
/// `is_method` wiring).
pub fn is_method(name: &str) -> bool {
    METHODS.contains(&name)
}

/// `stdlib::call` entry for `stream/promises.<method>`.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "finished" => finished(args),
        "pipeline" => pipeline(args),
        _ => return None,
    })
}

/// Compile a single JS expression and run it on the current host, returning its
/// completion value (re-entrant-safe; mirrors `util`'s `run_completion`).
fn run_completion(src: &str) -> Result<Value, String> {
    let prog = crate::compile_completion(src)?;
    let chunk = crate::load_merged(prog);
    crate::host::run_chunk_on(chunk)
}

// `finished(stream[, options])` → a Promise. The incoming args (stream, and an
// optional options object) are forwarded to the callback-based `stream.finished`
// with an appended settling callback; the existing impl picks the last callable as
// its callback and `args[0]` as the stream.
const FINISHED_SRC: &str = "(function(){\n\
  var stream = require('stream');\n\
  return function(){\n\
    var args = Array.prototype.slice.call(arguments);\n\
    return new Promise(function(resolve, reject){\n\
      args.push(function(err){ if (err) reject(err); else resolve(); });\n\
      stream.finished.apply(stream, args);\n\
    });\n\
  };\n\
})";

// `pipeline(source, ...transforms, destination)` → a Promise that resolves when the
// chain completes (rejects on error). Forwards to the callback-based
// `stream.pipeline` with an appended settling callback.
const PIPELINE_SRC: &str = "(function(){\n\
  var stream = require('stream');\n\
  return function(){\n\
    var args = Array.prototype.slice.call(arguments);\n\
    return new Promise(function(resolve, reject){\n\
      args.push(function(err, val){ if (err) reject(err); else resolve(val); });\n\
      stream.pipeline.apply(stream, args);\n\
    });\n\
  };\n\
})";

/// `stream.promises.finished(stream[, options])` → a Promise settled on the
/// stream's first terminal event (or rejected on `error`).
fn finished(args: &[Value]) -> Result<Value, String> {
    let factory = run_completion(FINISHED_SRC)?;
    crate::host::invoke(&factory, args.to_vec(), None)
}

/// `stream.promises.pipeline(...streams)` → a Promise resolved when the piped chain
/// completes (or rejected on `error`).
fn pipeline(args: &[Value]) -> Result<Value, String> {
    let factory = run_completion(PIPELINE_SRC)?;
    crate::host::invoke(&factory, args.to_vec(), None)
}
