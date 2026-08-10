# node-js — known gaps and unimplemented behavior

## Node core-module coverage
Implemented natively (verified vs node v26): `assert`(+`/strict`), `buffer`,
`child_process` (exec/spawnSync/execSync; `spawn` is sync-backed, not a live
streaming ChildProcess), `console`, `crypto` (hashes/hmac), `dns` (lookup/resolve
via std), `diagnostics_channel`, `events`, `fs`, `http`, `net`, `os`, `path`
(+`/posix` +`/win32` — both flavors are a faithful port of Node's `lib/path.js`,
differentially verified against the reference over a generated cross-product of
every method × posix/win32), `perf_hooks`, `process`, `punycode`,
`querystring`, `stream`,
`string_decoder`, `timers`(+`/promises`), `tty`, `url` (both the WHATWG `URL`
and the legacy `parse`/`format` API — the latter a faithful port of Node's
`Url.prototype.parse`/`.format`, differentially verified against the reference
over a generated cross-product of inputs × `parseQueryString` ×
`slashesDenoteHost`, dumping every `Url` field),
`util`(+`/types`), `v8`
(serialize = JSON, not V8 binary; heap stats are a shim), `async_hooks`
(AsyncLocalStorage sync-only; hooks are no-ops), `zlib`.

`process.emitWarning` writes to stderr in Node's format
(`(node:PID) [CODE] Name: message`, plus the one-time
`(Use \`node --trace-… ...\`)` hint). `url.parse` emits `DEP0169` through it.

`--no-warnings` and `--no-deprecation` behave as in Node (the warning is
suppressed entirely). `--trace-warnings` / `--trace-deprecation` are ACCEPTED
and suppress the one-time hint, but do not add the creation stack Node prints
under them — node-js keeps no allocation-site stack for a warning:

| command | node v26.7.0 | node-js |
| --- | --- | --- |
| `--trace-warnings -e 'process.emitWarning("m")'` | `(node:N) Warning: m` then a stack | `(node:N) Warning: m`, no stack |

Known-but-UNIMPLEMENTED: `inspector` and `wasi`. `require()` returns a namespace
so import-then-conditional code loads, and calling a method throws
`Error: <mod>.<method> is not implemented in node-js` — honest, never a silent
fake. Verified: `require("inspector").url()` and `require("wasi").WASI()` each
throw exactly that.

**This list used to name thirteen modules. Eleven of them are implemented and
the entry was simply never retired** — a doc-only staleness, found by running
every module in it. Each was checked against the reference, not merely
required:

| module | evidence it is implemented |
| --- | --- |
| `tls` | `tls.getCiphers().length > 0` is `true` on both; the namespace carries 9 real members |
| `https` | `https.get("https://example.com", …)` reports `status 200` on both |
| `http2` | server side works (`createSecureServer` validates `key`/`cert`); only `http2.connect` (the CLIENT) throws |
| `worker_threads` | real OS threads — a `new Worker(__filename, {workerData})` round trip is byte-identical to node |
| `cluster` | `cluster.fork()` really forks; primary and worker both run |
| `dgram` | real UDP — bind, send to self, receive `message` works on both |
| `trace_events` | `createTracing({categories})` + `enable`/`getEnabled` match node exactly |
| `domain` | `d.run()` executes and `on('error')` catches a thrown error |
| `repl` | `repl.start({input, output})` returns a server object on both |
| `readline` | `createInterface({input, output})` returns an interface on both |
| `dns/promises` | full 21-member API; `lookup("localhost")` matches node. The old "(use `require('dns').promises`)" workaround is obsolete |

The uniform `<mod>.<method> is not implemented in node-js` message applies to
`inspector` and `wasi` only. The implemented modules raise their own errors for
bad arguments (`tls.createServer()` → `TypeError: tls.createServer requires an
options object with 'key' and 'cert'`), which is Node-shaped behavior rather
than a stand-in.

`vm` is NOT in that list — it is implemented (`src/stdlib/vm.rs`), because the
engine has a real runtime source evaluator. `runInThisContext`, `Script`,
`compileFunction` and `runInNewContext` all compile and run genuine source; what
`vm` does NOT provide is context ISOLATION, since there is only one global heap.
That limitation is stated in the module's own docs and is not the same thing as
being unimplemented.

Two ECMAScript globals are absent entirely, and reference as `ReferenceError`
rather than pretending:

- **`Proxy`.** `Reflect` is complete (all 13 methods), but `Proxy` needs
  interception hooks in every property funnel — `get_property`,
  `set_property`, `has_property`, both delete paths, own-key enumeration, call
  and construct — each of which has to invoke a user trap from inside a host
  borrow. That is a structural change to the object model, not an addition to
  it, so it is not attempted here rather than shipped half-working.
- **`Intl`.** Needs ICU (locale-aware number/date/collation data). There is no
  honest subset: a `Intl.NumberFormat` that only handles `en-US` would give
  wrong answers for every other locale instead of an error.

`async_hooks.AsyncResource` carries a real id graph. Each `new AsyncResource()`
takes the next monotonically increasing `asyncId` (from 2 — Node reserves 1 for
the root context) and records the creating context as its `triggerAsyncId`;
`runInAsyncScope` installs that pair as the current execution context for the
duration of the call, so `executionAsyncId()` inside reports the resource and a
resource constructed there inherits it as its parent. Nesting builds a real
parent chain. `bind` (instance and static) returns the function bound to its
`thisArg`, and `emitDestroy()` returns `this` without destroying anything.

The graph covers ONLY resources this module creates. node-js does not instrument
timers, promises or sockets, so `executionAsyncId()` inside a `setTimeout`/
`.then` callback reports the root context (1), not a per-callback id, and
`createHook` callbacks still never fire.

## Promise resolution, async iteration and unhandled rejections

- **Thenable assimilation.** Resolving a promise with any object carrying a
  callable `then` adopts it through `NewPromiseResolveThenableJob`
  (ECMA-262 27.2.1.3.2) instead of fulfilling WITH the object — so `new
  Promise(r => r(thenable))`, `Promise.resolve(thenable)`, a `.then` handler
  RETURNING a thenable, and `await thenable` all deliver the settled value. A
  native promise is a thenable too, so `.then`-chaining one costs the same two
  microtask ticks it does in V8; `await` keeps V8's one-tick fast path.
- **`AsyncGeneratorYield` awaits.** `yield v` in an `async function*` awaits `v`
  before the step promise settles, so `yield somePromise` hands the consumer the
  RESOLVED value.
- **`[[AsyncGeneratorQueue]]`.** Overlapping requests on an async generator queue
  and resume the body one at a time, so their results settle in request order.
  `asyncGen.next()` returns a promise (it used to return a bare `{value, done}`
  record, so `it.next().then(...)` threw). `.return()` and `.throw()` enqueue
  too (27.6.3.6 `AsyncGeneratorEnqueue` records a *completion*, not just a sent
  value) — they used to unwind the body on the spot, so a `.return()` issued
  while an earlier `.next()` was suspended on an internal `await` terminated the
  generator underneath it and that `.next()` wrongly reported `{done: true}`;
  an uncaught `.throw(e)` also threw synchronously instead of rejecting the
  promise it returned. A `.return()` costs one extra microtask: its value is
  Awaited before the body sees it, via `AsyncGeneratorUnwrapYieldResumption`
  (27.6.3.7) at a `yield` and `AsyncGeneratorAwaitReturn` (27.6.3.9) otherwise.
  `.next()` and `.throw()` resume inline.
- **Unhandled rejections are fatal**, matching Node's default
  `--unhandled-rejections=throw`: at each microtask checkpoint a promise that
  settled rejected with no handler is reported on stderr and the process exits
  non-zero. `process.on('unhandledRejection', fn)` intercepts it —
  `process.on`/`once`/`off`/`removeListener`/`removeAllListeners`/`listeners`/
  `emit` keep a real listener table instead of discarding registrations. (`once`
  was in that list while behaving exactly like `on` — the listener fired on
  every emit and stayed registered. It deregisters before running now.)

## Abrupt completions (`break` / `continue` / `return` / `throw`)

A `try`/`catch`/`finally` body runs as its own chunk, so a jump leaving it
travels as a signal that the instruction after the `TRY` re-dispatches. Three
things that got wrong are now right, and are pinned by `tests/es_parity.rs` plus
the fuzzer's `unwind` mode:

- A `finally` that completes abruptly (`return`/`break`/`continue`) REPLACES the
  try/catch block's completion, discarding a pending exception (14.15.3).
- `break` and `continue` resolve their targets INDEPENDENTLY: a `switch` (or a
  labeled block) catches `break` but not `continue`. Requiring both to exist in
  the same chunk made a `break` out of a `try` inside a loop-free `switch` fall
  through to "propagate outward", silently halting the program.
- The signal's landing pad now closes the block scopes and for-of/for-in
  iterators it jumps past, which the compiler-resolved `break`/`continue` always
  did. A leaked scope left the frame standing in a dead child env, so the NEXT
  `let`/`const` at that level bound somewhere later closures could not see
  (`for (let i…) { let z = i; try { break; } finally {} } const f = 1;
  function g() { return f; }` threw `ReferenceError: f is not defined`).
- A coroutine body's own frame is now marked, so a top-level `let`/`var` inside
  an `async` function or generator is a LOCAL. It used to be declared as a
  global (the module frame was identified by frame COUNT, and a coroutine's
  swapped-in context holds exactly one frame), so two interleaved activations of
  the same async function shared one binding.

## Runtime source evaluation (`new Function`, `eval`, `vm`)

Source compiled at runtime goes through ONE evaluator, `crate::eval_in_global_scope`
(`compile_completion` → `load_merged` → `host::run_chunk_in_global_scope`). Its
callers are the CommonJS module wrapper, `vm.runInThisContext`/`Script`/
`compileFunction`, `new Function`/`Function(...)`, and the internal JS factories
(`util.promisify`, `stream/promises`, `stream/consumers`, `performance.timerify`,
`module.builtinModules`).

`new Function(p…, body)` reproduces V8's synthesized source exactly — measured on
node v26.7.0, `new Function('a','b','return a+b').toString()` is
`"function anonymous(a,b\n) {\nreturn a+b\n}"` with `.name === 'anonymous'`,
while `vm.compileFunction('return a+b', ['a','b']).toString()` is
`"function (a, b) {\nreturn a+b\n}"` with `.name === ''`. That text is retained
per function, so `Function.prototype.toString` reports it; ordinary functions
keep no source (the compiler records no spans) and still render
`function <name>() { [code] }`.

`eval` distinguishes the two call forms the spec distinguishes. A DIRECT eval —
the literal `eval(...)` form, which is the only one reaching `host::call_named` —
evaluates in the caller's scope; `(0, eval)(src)`, `const e = eval; e(src)` and
every other value-call form is an INDIRECT eval and evaluates in the global
scope. A user binding named `eval` shadows the intrinsic and wins.

Two limits remain. `this` inside a dynamic function is `undefined` rather than
`globalThis` — that is the general sloppy-mode `this` gap below, not specific to
dynamic code. And `vm`'s contexts are not isolated (see `src/stdlib/vm.rs`).

## Native constructors: real prototypes and ES5 subclassing

A native stdlib constructor (`StringDecoder`, `Hash`, `URLSearchParams`, …) has a
real `.prototype` object, built on first read from the same
`stdlib::instance_method_lists` table a method *read* consults, so the prototype
cannot advertise a name the dispatcher does not implement. `Ctor.prototype` used
to read `undefined` for everything outside the hand-written `is_builtin_ctor`
list. (`Hash` was in that sentence for a round while `instance_method_lists`
listed only its twin `Hmac`, so `crypto.Hash.prototype` really did read
`undefined` — the table now carries both, which is the whole point of building
the prototype from it.)

`NativeCtor.call(obj, …)` — ES5 "constructor stealing" — initializes `obj`
instead of returning a fresh instance, but ONLY when `obj` already inherits from
that constructor's prototype. Without that guard `Date.call(x)` and
`Buffer.call(x)`, which in JS ignore `this`, would start mutating `x`.

`Buffer.poolSize` (65536) is a DATA property rather than a method and was
missing from both tables, so it read `undefined`; it is served from
`stdlib::constant` now.

`Buffer.copyBytesFrom` is the one `Buffer` static still missing, and is
deliberately absent from `buffer::STATIC_METHODS` rather than advertised: it
copies a typed array's raw bytes with `offset`/`length` counted in ELEMENTS, and
typed arrays are stored here as a `@@elems` array of numbers, so it needs a
per-kind little-endian serializer for all nine element kinds.

## `process.exit` really exits; `chdir` and `kill` really act

`process.exit([code])` terminates immediately — nothing after the call runs, and
no pending timer, microtask or `nextTick` fires. It used to return `undefined`
and let execution continue, which broke the idiom `if (done) { srv.close();
process.exit(0); }`: Node needs no `return` there, so real code does not write
one, and the next statement ran anyway. `process.chdir(dir)` and
`process.kill(pid[, sig])` were no-ops for the same reason and now perform the
real syscall, throwing on failure. `process.setSourceMapsEnabled` stays a no-op,
which is its whole contract here since node-js emits no source maps.

Signal names map through `libc` constants, not a literal table: numbering
differs between macOS and Linux above `SIGTERM` (`SIGUSR1` is 30 on Darwin, 10
on Linux). `os.constants.signals` still reports the Darwin numbers and is the
remaining literal table.

## Sloppy-mode `this` in a plain CALL is `undefined`, not `globalThis`

A plain (unbound, non-method) function call binds `this` to `undefined` here; in
sloppy mode Node binds it to `globalThis` and boxes a primitive `this`
(10.2.1.2 `OrdinaryCallBindThis`). Measured on node v26.7.0,
`function f(){ return typeof this } ; [f(), f.call(null), f.call(5)]` is
`['object','object','object']` there and `['undefined','object','number']` here.

Scoped to a CALL. The TOP-LEVEL `this` was `undefined` too and is now correct at
all three entry points — see the FIXED section below.

## FIXED — strings are indexed by UTF-16 code unit

Strings used to be indexed by code POINT, so every index agreed with node
across the whole BMP and shifted by one for each astral character past it.
They are now indexed by UTF-16 code unit, which is what JS counts. Measured on
node v26.7.0 with `a = "𝒳"`:

| expression | node v26.7.0 | node-js (was) | node-js (now) |
| --- | --- | --- | --- |
| `a.length` | `2` | `1` | `2` |
| `a.charCodeAt(0)` | `55349` | `119987` | `55349` |
| `a.charCodeAt(1)` | `56499` | `NaN` | `56499` |
| `a.codePointAt(0)` | `119987` | `119987` | `119987` |
| `a.split("").length` | `2` | `1` | `2` |
| `"ab𝒳cd".indexOf("c")` | `4` | `3` | `4` |
| `"𝒳".padStart(3, "-")` | `"-𝒳"` | `"--𝒳"` | `"-𝒳"` |
| `String.fromCharCode(0x1D4B3)` | `"\u{D4B3}"` | `"𝒳"` | `"\u{D4B3}"` |
| `/c/.exec("ab𝒳cd").index` | `4` | `3` | `4` |

`String.fromCodePoint` was listed in the builtin name table but had no dispatch
arm, so calling it threw `is not a function`; it is implemented now, including
the `RangeError` on a non-code-point argument.

`src/utf16.rs` is the single UTF-8 ⇄ UTF-16 boundary; `String.prototype`'s
index arms, `s[i]`, `.length`, `padStart`/`padEnd`, and the RegExp
`.index`/`lastIndex` path all translate through it. A `U16Index` newtype keeps
a code-unit index from being passed where the regex engine wants a byte offset.
`[Symbol.iterator]` still iterates code POINTS — that is what the spec says, so
`[..."𝒳"]` is one element even though `"𝒳".length` is 2.

**Remaining boundary — a value containing an unpaired surrogate.** A Rust
`String` cannot hold one (`char` excludes `U+D800..=U+DFFF`), and both
`fusevm::Value::Str` and the host heap's `JsObj::Str` are `String`. An
operation that cuts a surrogate pair in half therefore yields `U+FFFD` where
node yields the lone surrogate:

| expression | node v26.7.0 | node-js |
| --- | --- | --- |
| `console.log("𝒳".charAt(0))` | `ef bf bd` | `ef bf bd` — agrees |
| `"𝒳".charAt(0).length` | `1` | `1` — agrees |
| `"𝒳".charAt(0).charCodeAt(0)` | `55349` | `65533` |
| `JSON.stringify("𝒳".charAt(0))` | `"\ud835"` | `"�"` |
| `"𝒳".slice(0,1) + "𝒳".slice(1,2)` | `"𝒳"` | `"��"` |
| `console.log("𝒳".split(""))` | `[ '\ud835', '\udcb3' ]` | `[ '�', '�' ]` |

Note the first two rows: node itself writes `U+FFFD` when a lone surrogate
reaches stdout (`node -e 'process.stdout.write("𝒳".charAt(0))' | xxd`), and a
lone surrogate and `U+FFFD` are both one code unit, so PRINTING is
byte-identical and every surrounding index still lines up. Only re-inspecting
an extracted half differs. Closing that last gap means replacing `String` with
a WTF-8 buffer inside `fusevm` and all of `src/stdlib`, which the pinned
`fusevm` dependency does not allow.

The UTF-8 arm of `Buffer.byteLength` / `Buffer.from(s)` and `StringDecoder`
across a split multi-byte sequence work in UTF-8 bytes and were never affected.
The rest of the Buffer surface *was*, and this file said otherwise until the
sweep below — see the next section.

## FIXED — Buffer encodings that count code units, and the argument forms that reach them

Three of Node's buffer encodings are defined over the string's UTF-16 CODE
UNITS, not over its UTF-8 bytes, so the UTF-16 sweep above did not finish at
`String.prototype`: `utf16le` (and its `ucs2` spellings) is those units written
little-endian, and `latin1`/`ascii` take the low byte of each unit. `utf16le`
had no arm at all and fell through to UTF-8 — silent corruption rather than an
error — and `latin1` encoded one byte per code POINT. Measured on node v26.7.0:

| expression | node v26.7.0 | node-js (was) | node-js (now) |
| --- | --- | --- | --- |
| `Buffer.byteLength("abc","utf16le")` | `6` | `3` | `6` |
| `Buffer.from("abc","utf16le").toString("hex")` | `610062006300` | `616263` | `610062006300` |
| `Buffer.from("61006200","hex").toString("utf16le")` | `"ab"` | `"a\0b\0"` | `"ab"` |
| `Buffer.from("A\u00ff\u0100\u{1D4B3}","latin1").toString("hex")` | `41ff0035b3` | `41ff00b3` | `41ff0035b3` |
| `[...Buffer.from([65,255,128]).toString("ascii")]` codes | `65,127,0` | `65,255,128` | `65,127,0` |
| `Buffer.from([251,255,190]).toString("base64url")` | `-_--` | `+/++` | `-_--` |
| `Buffer.from("-_-_","base64").toString("hex")` | `fbffbf` | `""` | `fbffbf` |
| `new StringDecoder("ucs2").encoding` | `utf16le` | `ucs2` | `utf16le` |

`base64url` was an alias of `base64` in both directions. Encoding it must use
the URL-safe alphabet with the padding dropped; decoding must accept `-_` under
*either* name, and dropping them produced an EMPTY buffer because an
unrecognized character is skipped rather than rejected.

The argument forms that select an encoding were being ignored outright, so even
the encodings that did work were unreachable through most of the API:

| expression | node v26.7.0 | node-js (was) | node-js (now) |
| --- | --- | --- | --- |
| `Buffer.from("abcdef").toString("utf8",1,3)` | `"bc"` | `"abcdef"` | `"bc"` |
| `Buffer.from("abcdef").indexOf("b",2)` | `-1` | `1` | `-1` |
| `Buffer.from("abcdef").lastIndexOf("b",0)` | `-1` | `1` | `-1` |
| `Buffer.alloc(6,0x2e).write("ZZZZ",1,2)` | `2`, `.ZZ...` | `4`, `.ZZZZ.` | `2`, `.ZZ...` |
| `Buffer.alloc(6).write("ab","hex")` | `1`, `ab0000000000` | `2`, `616200000000` | `1`, `ab0000000000` |
| `Buffer.alloc(4).fill("ff","hex").toString("hex")` | `ffffffff` | `66666666` | `ffffffff` |
| `Buffer.from([1,2,3,4]).swap16().toString("hex")` | `02010403` | `TypeError` | `02010403` |

`write` truncates at a CHARACTER boundary — node reports 2, not 4, for
`Buffer.alloc(4).write('é€')`, dropping the 3-byte `€` whole rather than
half-writing it — and a `write` offset past the end is a `RangeError`, not a
silent no-op. `fill`'s overload resolution is node's own: a STRING in the
`offset` slot is the encoding *and resets the range to the whole buffer*, so
`fill('41','hex',1,3)` fills all of it.

`StringDecoder` now buffers every encoding that has a chunk boundary, not just
UTF-8: UTF-16LE holds an odd trailing byte and a trailing high surrogate, and
base64 holds up to two bytes so it emits whole 3-byte groups
(`AQID`/`BA==`, never `AQI=`/`AwQ=`). Its `encoding` property reports the
canonical name (`ucs2` → `utf16le`, `UTF-8` → `utf8`).

## FIXED — relational comparison and default `sort` order by code unit

The same code-unit/code-point split reaches string ORDER, which the UTF-16
sweep did not cover: 7.2.13 IsLessThan and 23.1.3.30.2 SortCompare both compare
code units, and Rust's `str: Ord` is UTF-8 byte order (code-point order). A
surrogate is `0xD800..0xE000`, so every astral character sorts BELOW every BMP
character from `U+E000` up, and the two orders disagree on exactly those pairs.
Measured on node v26.7.0:

| expression | node v26.7.0 | node-js (was) | node-js (now) |
| --- | --- | --- | --- |
| `"\u{1D4B3}" < "￿"` | `true` | `false` | `true` |
| `["￿","\u{1D4B3}","","a"].sort()` | `a,𝒳,,￿` | `a,,￿,𝒳` | `a,𝒳,,￿` |

`utf16::cmp_units` is that comparison, and both call sites now use it.
`localeCompare` is deliberately NOT routed through it — it is documented as an
ASCII approximation of ICU collation and needs real collation data, not a
different code-unit order.

## FIXED — `escape` / `unescape` and `String.prototype.isWellFormed` / `toWellFormed`

`escape`/`unescape` (Annex B.2.1) were absent, so calling either was a
`ReferenceError`. They are code-UNIT encoders, which is what separates `escape`
from `encodeURIComponent`: `escape("\u{1D4B3}")` is `"%uD835%uDCB3"`, the two
surrogates, where `encodeURIComponent` gives the UTF-8 bytes `%F0%9D%92%B3`.
`unescape` never throws — a `%` that starts no valid escape passes through
(`unescape("%u0041%42%zz%2")` is `"AB%zz%2"`).

`String.prototype.isWellFormed`/`toWellFormed` (ES2024) were absent
(`TypeError: isWellFormed is not a function`). Every string this runtime can
hold is well-formed by construction — a Rust `char` excludes
`U+D800..=U+DFFF` — so `isWellFormed` is `true` and `toWellFormed` is the
identity, both exact for every representable value. The one case node answers
differently is a surrogate half extracted by `charAt`/`slice`, which is already
`U+FFFD` here: the lone-surrogate boundary above, not a separate gap.

## FIXED — `globalThis` is one object, and top-level `this` is bound

`globalThis` used to mint a FRESH object on every read, so it failed the two
things an identity is for: `globalThis === globalThis` was `false`, and
`globalThis.x = 1` was unreadable through the next `globalThis.x`. It is a
singleton on the host now, and `global` is an alias for the same object rather
than a `ReferenceError`.

Top-level `this` was `undefined` at every entry point. Node answers
`module.exports` from a FILE (a CommonJS module) and `globalThis` from `-e` and
from stdin (a Script); all three now agree, so `this.x = 1` at module scope
populates the exports object instead of throwing. Measured on node v26.7.0 with
`console.log(this === globalThis, this === module.exports)`:

| entry point | node v26.7.0 | node-js (was) | node-js (now) |
| --- | --- | --- | --- |
| `node f.js` | `false true` | `false false` | `false true` |
| `node -e` | `true false` | `false false` | `true false` |
| `node -` | `true false` | `false false` | `true false` |

**Remaining:** `globalThis` is still not backed by the global SCOPE. A top-level
`var y = 2` does not appear as `globalThis.y`, and `globalThis.x = 1` does not
create a bare readable binding `x` — the two live in separate tables
(`JsHost.globals` versus the `globalThis` object). A property written through
`globalThis` is readable through `globalThis`, which is what the identity fix
bought; joining the two tables is a separate change. `globalThis.Error` and the
other builtin names likewise read `undefined`, since the builtins are resolved by
name at the `GetLocal` site rather than stored as properties of a global object.

## Entry points: `node f.js` vs `node -e` vs `node -` / piped stdin

These are three DIFFERENT entry points, and Node reports different values at
each of them. A harness that only ever exercises one cannot see a regression in
the others, and an expectation captured at the wrong one is measuring something
it did not mean to. The table below is the full observable set, measured on node
v26.7.0; every row marked **agrees** is now pinned by a test.

| observable | `node f.js` | `node -e src` | `node -` / piped | node-js |
| --- | --- | --- | --- | --- |
| `typeof module` / `exports` | `object` | `object` | `object` | agrees |
| `exports === module.exports` | `true` | `true` | `true` | agrees |
| `__filename` | resolved abs path | `[eval]` | `[stdin]` | agrees |
| `__dirname` | its directory | `.` | `.` | agrees |
| `module.id` | `.` | `[eval]` | `[stdin]` | agrees |
| `module.path` | its directory | `.` | `.` | agrees |
| `Object.keys(module)` | `id,path,exports,filename,loaded,children,paths` | same | same | agrees |
| `process.argv[1]` | resolved abs path | *absent* | `-` | agrees |
| `process.execArgv` | runtime flags | flags + `-e` + src | runtime flags | agrees |
| `require.main === module` | `true` | `false` | `false` | **`require.main` absent** |
| top-level `this` | `module.exports` | `globalThis` | `globalThis` | agrees |
| top-level `arguments` | the wrapper's 5 | *undefined* | *undefined* | **undefined at all three** |
| `arguments.callee` in a function | the function | same | same | **`undefined`** |
| stack frame file | `file:L:C` | `[eval]:L:C` | `[stdin]:L:C` | **no `file:line:col`** |

A row that used to sit in this table said node-js was **strict at all three**
entry points. That was never true and is removed: by every testable strict-mode
restriction node-js is SLOPPY, exactly as Node is. An implicit global assignment
succeeds, `01` is a legal octal literal, duplicate parameter names are accepted,
and a write to a frozen object is silently discarded — all four matching node.
The one strict-SHAPED behavior was `this === undefined` in a plain call, which
is its own gap (see the sloppy-mode section above) rather than a mode. `with`
is rejected, but with a generic parse error, which is a parser gap and not a
strict-mode rejection.

The remaining rows follow from one thing: node-js runs the entry
source directly rather than through the CommonJS wrapper function.
`module`/`exports`/`__filename`/`__dirname` are
installed as globals per entry point (`module::install_entry_globals`), which
fixes what packages actually read — a UMD header's
`typeof module !== 'undefined' && module.exports` now takes the CommonJS branch
at every entry point instead of the browser branch — but top-level `this`,
`arguments` needs the wrapper itself. See the two sections below for what else
the missing wrapper costs.

A `require`d module gets the same seven-key `module` object — `id`, `path`,
`exports`, `filename`, `loaded`, `children`, `paths`, in that order — where it
used to carry `exports` alone, so `module.id`/`module.filename`/`module.path`
were `undefined` inside every dependency. A required module's `id` IS its
absolute filename; only the entry module's is `.`. `loaded` flips to `true` once
the body returns. Still missing on the loader side: `module.children` is always
empty (populating it needs the loader to thread the REQUIRING module through
`require`, which it does not), and `require.main` and `require.cache` are absent.

`__filename` is the entry script's REALPATH, matching Node's `toRealPath` on the
main module, while `process.argv[1]` keeps the spelling that was passed —
`node link/a.js` through a symlinked directory reports the link in `argv[1]` and
the target in `__filename`. `require.resolve` normalizes the joined path, so
`require('./d.js')`, `require('././d.js')` and `require('../<dir>/d.js')` from
one directory are a single cache entry rather than three. (An earlier wording
here contrasted `'./d.js'` with a BARE `'d.js'`; that is not the same
comparison — a bare specifier is a package lookup in both runtimes and resolves
to neither, `MODULE_NOT_FOUND` on each side.)

Two harnesses in this repo compare against DIFFERENT entry points on purpose:
`parity-scripts/run.sh` runs each corpus case as a script FILE, and
`src/bin/parity_fuzz.rs` runs each generated case through `-e`. The split is
coverage rather than contamination, but for two rounds it also meant NOBODY
measured this table: no generator emitted a bare `this`, `globalThis`,
`arguments` or `require.main`, so "unreachable by both" was mistaken for
"covered by one". Top-level `this` was wrong at all three entry points the whole
time. Both sides are now driven deliberately — the fuzzer's `entry` mode emits
the entry-point-INVARIANT relations plus the `-e`-specific ones, and
`parity-scripts/lang/27_top_level_this.js` carries the FILE answers, which the
fuzzer cannot reach. A case added to either that touches an entry-point-VARIANT
row is still measuring that harness's entry point, not "node".

## `node -e` evaluates a Script, not a CommonJS module

Node wraps a `.js` FILE in the CommonJS wrapper, which makes a top-level `return`
legal; under `-e` it evaluates a Script, where the same `return` is a
`SyntaxError`. node-js accepts a top-level `return` in both, so `node -e
'return'` exits 0 here and 1 in Node.

The same difference has a second, opposite face: node-js evaluates a FILE with
Script semantics too, so a file's top-level `var` is a global here and a module
local in Node. Measured on node v26.7.0 with
`var mv = 1; console.log(new Function("return typeof mv")())`, a file prints
`undefined` there and `number` here, while `node -e` with the same source prints
`number` in both. Dynamically compiled source therefore sees an entry FILE's
top-level bindings here and does not in Node. Closing it means running the entry
file through the CommonJS wrapper, as Node does.

`err.stack` names the REAL call chain — one `    at <function>` line per live
frame — but carries no `file:line:column` (per-frame line tracking is only
enabled under `--dap`) and none of Node's internal module-loader frames. So
`.stack` is diagnosable but can never be byte-identical to V8's, and no parity
script asserts its text. `console.log(err)` renders the stack followed by any
own property a script added (`Error: x\n    at f { code: 'C' }`), the shape V8
uses — it no longer prints an object literal exposing the internal
`message`/`stack` slots.

`JSON.parse` reproduces V8's full family of failure messages, including the
context-window rule for the default form (the whole input quoted when it is 20
characters or shorter, otherwise a 10-character window either side of the error
elided with `...`), the positional forms, and JSON's number grammar (`01` parses
as `0` and then reports the stray digit, exactly as V8 does). Every case in
`parity-scripts/data/22_json_parse_errors.js` matches byte-for-byte; the count
is whatever that file currently holds and is not restated here, because a number
typed into prose goes stale the first time a case is added.

**Except the ESCAPE family, which is missing and fails OPEN.** V8 raises
`Bad escaped character in JSON at position N` and `Bad Unicode escape in JSON at
position N`; node-js has neither, and the malformed escape is silently accepted
with the character DROPPED — silent corruption rather than a rejection. Measured
on node v26.7.0:

| input | node v26.7.0 | node-js |
| --- | --- | --- |
| `JSON.parse('"\\x"')` | `SyntaxError: Bad escaped character in JSON at position 2` | `""` (parses, character lost) |
| `JSON.parse('"\\a"')` | `SyntaxError: Bad escaped character in JSON at position 2` | `""` |
| `JSON.parse('"\\uZZZZ"')` | `SyntaxError: Bad Unicode escape in JSON at position 3` | `""` |
| `JSON.parse('"\\u12"')` | `SyntaxError: Bad Unicode escape in JSON at position 5` | `SyntaxError: Unterminated string in JSON at position 7 (line 1 column 7)` |

The last row is doubly wrong: the message is the wrong family, and its
`position 7 (line 1 column 7)` breaks V8's own `column = position + 1`
convention. The corpus file has no bad-escape case, which is why this went
unreported while the rest of the family was being pinned.

A `Buffer` is a real `Uint8Array` subclass instance:
`Object.getPrototypeOf(buf) === Buffer.prototype` holds, that prototype is a
genuine object whose own `[[Prototype]]` is `Uint8Array.prototype`, and a
Buffer's own keys are its byte indices (`Object.keys`, `for…in`,
`getOwnPropertyNames`, `hasOwnProperty`, `getOwnPropertyDescriptor`, the `in`
operator and object spread all agree).

The full chain is the one Node has —
`Buffer.prototype → Uint8Array.prototype → %TypedArray%.prototype →
Object.prototype` — with the shared iteration methods on the `%TypedArray%`
intermediate, each element kind carrying its own prototype (so an `Int32Array`
does not claim `Uint8Array.prototype`), and the class side linked too:
`Object.getPrototypeOf(Buffer) === Uint8Array`. Every one of the nine element
kinds is prototype-linked, iterable, and answers `Symbol.toStringTag` with its
own brand. A Buffer inherits the typed-array methods it does not implement
itself (`every`/`some`/`map`/`filter`/`find*`/`reduce*`/`forEach`/`sort`/
`reverse`/`copyWithin`/`join`/`at`/`lastIndexOf`), and a derivation keeps the
receiver's type: `buf.map(f)` is a Buffer, `int32.map(f)` an `Int32Array`.
Anything that accepts a Buffer accepts any typed array — `Buffer.from`,
`Buffer.concat`, `buf.equals` and `buf.indexOf` share one byte-source helper,
and `Buffer.byteLength` reports the VIEW size (12 for three `Int32Array`
elements, not 3). A typed array sorts numerically, unlike `Array`.

Two divergences remain:

1. Node's `Buffer.prototype` methods are *enumerable*, so `for (k in buf)` in
   Node yields the ~100 prototype method names after the byte indices. node-js
   implements a subset of those methods and marks them non-enumerable, so
   `for (k in buf)` yields the indices only. Emitting our shorter list instead
   would advertise methods Node has that node-js does not.
2. **There is no shared backing store, so `.buffer` is absent and views do not
   alias.** `buf.buffer` is `undefined` where Node hands back the `ArrayBuffer`,
   and `buf.slice(1,3)`/`subarray(1,3)` copy where Node returns a window over
   the same memory (so writing through the result does not show up in the
   parent, and two views over one `ArrayBuffer` are independent). An
   `ArrayBuffer` here carries only a byte length, and a typed array's elements
   live in a per-object hidden array; making views alias means re-basing both
   onto one shared byte store and moving every `@@bytes`/`@@elems` call site
   (15 files, by `grep -rln`) onto it. That is a real refactor, not a patch — doing it
   partially would leave some views aliasing and others not, which is worse
   than the honest gap. `express.json()` does not depend on it (verified: a
   live POST round-trips byte-identically).

A builtin namespace (`require('buffer')`, `Buffer`, `fs`, …) enumerates the
members node-js ACTUALLY implements under `for…in` / `Object.keys` — not Node's
full export list. A package that clones a namespace key-by-key therefore gets
the working set rather than an empty object.


## Express (real npm package) — runs, serves HTTP, and parses request bodies
The real `express` — both the **4.x** and **5.x** lines — plus its dependency
tree loads and serves HTTP. Verified end-to-end against `node v26.7.0` with the
same app and the same requests, byte-comparing every response body:
`app.get`/routing/route params/query, `res.send`/`res.json`/`res.status`/
`res.type`, `app.listen`, a 404 fall-through handler, **and** the body parsers —
`express.json()` (object, array, UTF-8, empty, and the malformed-input error
path returning `entity.parse.failed`), `express.urlencoded()`,
`express.text()` and `express.raw()`. All byte-identical on both lines.

Express **5** cleared first, and the blockers were not the ones guessed before
it was tried:

1. **`async_hooks.AsyncResource` did not exist**, so `new AsyncResource(...)`
   threw `is not a constructor` inside `raw-body` and `on-finished`, which both
   wrap their callbacks with it.
2. **Builtin namespaces did not enumerate their members**, so `safer-buffer`'s
   `for (key in Buffer) Safer[key] = Buffer[key]` produced an empty object and
   `iconv-lite` then hit `isBuffer is not a function`.
3. **`String.prototype.indexOf` ignored its `fromIndex`**, so `body-parser`'s
   `parameterCount` never advanced past the first `&` and rejected every
   urlencoded body with `parameters.too.many`.

Express **4** shares almost none of that path and needed four more, each of
which failed earlier than the last:

4. **`new Function` did not exist.** `depd`'s `wrapfunction` builds its
   deprecation wrapper with it and `body-parser` calls that at module load, so
   `require('express')` itself threw `Function is not a constructor` — express 4
   never reached a single line of its own code.
5. **The EventEmitter surface on a request/socket/stream was missing
   `listeners`**, which `unpipe` calls on every `express.json()` request.
6. **`StringDecoder.prototype` read `undefined`**, so `iconv-lite`'s internal
   codec — which adopts that prototype and initializes itself via
   `StringDecoder.call(this, enc)` — threw while building any decoder, i.e. on
   every request with a charset.
7. **`Buffer.allocUnsafeSlow` was missing**, which flipped `safe-buffer` onto its
   legacy `SafeBuffer` wrapper, whose `Buffer(arg, …)` call form then threw. That
   is the `Buffer` express 4's `res.send` uses, so every response died.

`Object.prototype.toString.call(buf)` is `[object Uint8Array]` (and every other
builtin exotic brands correctly), `ArrayBuffer.isView(buf)` is `true`, and
`buf.byteLength`/`byteOffset`/`BYTES_PER_ELEMENT`/`set` exist. A `Buffer` is a
real `Uint8Array` subclass INSTANCE — see the Buffer section above; the earlier
`@@native`-tagged approximation is gone.

The same brand is readable as an ordinary property, not only through
`Object.prototype.toString`: `buf[Symbol.toStringTag]` is `'Uint8Array'`, and
every builtin that carries the symbol in Node reports it (`Map`, `Set`,
`Promise`, the typed arrays, `ArrayBuffer`, `DataView`, `WeakRef`,
`FinalizationRegistry`, `BigInt`, `Symbol`, generators, async/generator
functions, `Math`, `JSON`, `Reflect`, `URL`, `URLSearchParams`, `TextEncoder`,
`TextDecoder`) while the legacy ones that do not (`Array`, `Function`, plain
objects, `Date`, `RegExp`, `Error`) read `undefined`. The brand and the symbol
are computed from one function, so they cannot drift apart.

The end-to-end check for all of this is a live POST, not an assertion about
prototypes: real `express.json()` reads the request body off the socket into a
Buffer, hands it through `raw-body`/`iconv-lite` to `JSON.parse`, and the route
echoes the PARSED OBJECT back. Under `node v26.7.0` and under node-js the same
app answers `{"got":{"a":1,"b":[2,3],"c":"ü"},"keys":["a","b","c"],"isArr":false,
"sum":2}` byte-for-byte, and `/raw` reports `isBuf: true, isU8: true, protoOk:
true` for a body that came off the wire.


node-js is JavaScript lowered to fusevm (bytecode VM + Cranelift JIT), with a
JsHost object heap. It runs a real subset of JavaScript correctly, verified
byte-for-byte against system `node` on the example corpus (`tests/parity.rs`) and
via the differential fuzzer (`parity-fuzz`, 12000+ mixed cases clean against
`node v26.5.0`). This file is the honest list of what is **not** yet covered, so
nobody mistakes a gap for a bug fixed.

The `parity-fuzz` generator deliberately stays within the implemented surface:
its contract is "find real bugs in shipped features," so it does not emit the
constructs below. Each is a genuine gap, not something the harness hides.

## Implemented since the original object-model work (now fuzzed, not gaps)

These were previously listed as unimplemented and are now covered — with
dedicated fuzzer modes (`class`, `generator`, `mapset`, `proto`, `async`,
`bigint`, `regex`) that track the surface:

- **`BigInt`** — the `10n`/`0xffn`/`0o..n`/`0b..n` literal, a heap
  `JsObj::BigInt(num_bigint::BigInt)` with `typeof === "bigint"`. Arithmetic
  `+ - * / % **` (division/`%` truncate toward zero), bitwise `& | ^ << >>`
  (arbitrary width; `>>>` throws as in JS), comparisons (`<`/`>` numeric, `==`
  loose-coerces across Number, `===` false across types). **Mixing a BigInt with
  a Number in arithmetic throws the exact Node `TypeError: Cannot mix BigInt and
  other types, use explicit conversions`;** unary `+` on a BigInt throws;
  `x++`/`x--` stay BigInt (type-preserving). Formatting: `String(10n) === "10"`,
  `console.log(10n)` → `10n`, `(255n).toString(16)`, `JSON.stringify(1n)` throws.
  The `BigInt(x)` constructor + `BigInt.asIntN`/`asUintN`. BigInt is a valid
  Map/Set key.
- **Regular expressions** — `/pat/flags` literals (with the regex-vs-divide
  disambiguation) and `new RegExp(source[, flags])`, backed by the Rust `regex`
  crate. `re.test`/`re.exec` (with `.index`/capture groups/named `.groups`/
  `lastIndex` under `g`/`y`), and the String methods `match`/`matchAll`/`replace`/
  `replaceAll`(with `$1`/`$&`/`` $` ``/`$'`/`$<name>`/`$$` + function replacers)/
  `split`/`search`. Flags `g`/`i`/`m`/`s`/`u`/`y`/`d`. **Rust `regex` is NOT a JS
  superset** — the exact supported subset and known divergences are in the
  dedicated section below.
- **Tagged templates** — `` tag`a${x}b` `` calls `tag(strings, ...values)` where
  `strings` is the cooked-quasi array carrying a `.raw` array; `String.raw`.
- **`for await (… of …)`** — async iteration over a `Symbol.asyncIterator`
  object (whose `.next()` returns a promise of `{value,done}`) or, as the sync
  fallback, over any iterable with each yielded value awaited.
- **`generator.return()` / `.throw()` run `finally`** — `.return(v)` and
  `.throw(e)` resume the suspended coroutine with an injected completion so a
  pending `try { … } finally { … }` executes (and a `try/catch` in the body can
  handle a `.throw`). A for-of `break` likewise closes the iterator (runs the
  generator's `finally` / calls a user iterator's `.return()`).
- **`util.inspect` array grouping** — `console.log` of an array with >6 elements
  uses Node's multi-column, right-aligned (for all-numeric/BigInt) grid, a
  faithful port of Node's `groupArrayElements` + single-line/one-per-line
  decision (`breakLength` 80, `compact` 3).

- **ES6 classes** — `class`/`extends`/`super(...)`/`super.method()`, constructor,
  instance + static methods, instance + static fields, `get`/`set` accessors,
  computed method names, private `#fields`/`#methods`, `new.target`, constructor
  object-return, static inheritance down the constructor chain, `class extends
  Error`.
- **Prototype chain** — `[[Prototype]]` delegation for property lookup;
  `Object.getPrototypeOf`/`setPrototypeOf`/`create`; `obj.__proto__` (read + the
  literal `{ __proto__: x }` form); `defineProperty`/`getOwnPropertyDescriptor`.
- **`instanceof`** (walks the chain; structural for builtin `Array`/`Object`/
  `Function`/`Map`/`WeakMap`/`Set`/`WeakSet`/`Promise`), **`in`** and
  **`hasOwnProperty`** respecting the chain.
- **`this` binding** — method calls, `fn.call`/`apply`/`bind`, arrow lexical
  capture, `new` binding, `new.target`.
- **Error hierarchy** — `Error`/`TypeError`/`RangeError`/`SyntaxError`/
  `ReferenceError`/`EvalError`/`URIError` as prototype-linked constructors with
  `.name`/`.message`/`.stack`, correct `instanceof`, throwable + catchable by type.
- **Map / Set / WeakMap / WeakSet** — construction from iterables, `get`/`set`/
  `has`/`delete`/`size`/`clear`, insertion-order iteration, `forEach`,
  `keys`/`values`/`entries`, spread, `for-of`.
- **Symbol** — `Symbol()`, `Symbol.for`/`keyFor`, well-known `Symbol.iterator`,
  symbol-keyed properties, `typeof sym === 'symbol'`.
- **Generators** — `function*`, `yield`, `yield*`, `.next(x)`/`.return()`,
  generator-as-iterable in `for-of` and spread (via `corosensei` stackful
  coroutines on the shared thread-local heap). `yield*` evaluates to the
  delegate's RETURN value and forwards the argument passed to `.next(x)`.
- **Async generators** — an `async function*` is its own async iterator, stepped
  lazily by `for await`. `await` and `yield` share one coroutine yielder, so an
  `await` suspension is tagged and settled by the driver rather than surfacing to
  the consumer; `yield*` inside an async generator uses the async protocol.
- **Iterators** — honoring `Symbol.iterator` in `for-of`/spread for user
  iterables; array/string/Map/Set/generator iterators with `.next()`.
- **Labeled statements** — `outer: for (...) { ... continue outer / break outer }`
  bind `continue`/`break` to the labeled loop target (compiler.rs). Verified
  against `node v26.5.0`: labeled `continue`/`break` retarget the correct loop.
- **Block scoping** — `let`/`const` live in the innermost block (`{ }`, a `switch`
  body, a `try`/`catch`/`finally` body, the catch parameter), while `var` and
  hoisted function declarations bind at function scope. `for`/`for-of`/`for-in`/
  `for await` with a `let`/`const` head create a PER-ITERATION environment, so a
  closure made in one pass captures that pass's value
  (`for (let i = 0; i < 3; i++) fs.push(() => i)` yields `0,1,2`).
- **Non-local control flow out of a `try`** — the host runs a `try`/`catch`/
  `finally` body as its own chunk, so `return`/`break`/`continue` crossing that
  boundary raise a signal that `SIG_UNWIND` re-dispatches after the `TRY` op
  (labeled targets across nested loops included).
- **`AggregateError`** — `new AggregateError(errors, message)` with `.errors`;
  `Promise.any` rejects with one carrying every reason.
- **`Error.prototype.toString`** — `String(err)` / `` `${err}` `` render
  `Name: message` (or just `Name` when the message is empty). An error raised by
  a core module carries Node's `.code` (`ERR_INVALID_ARG_TYPE`, …) and renders
  `Name [CODE]: message`, matching `internal/errors.js` `NodeError.toString`.
- **`-x ** y` is a `SyntaxError`** — the grammar allows only an
  UpdateExpression left of `**`, so `-x ** y` / `typeof x ** y` /
  `await x ** y` (and the BigInt forms) are rejected with Node's exact message
  rather than silently evaluated as `-(x ** y)`; `(-x) ** y`, `-(x ** y)`,
  `x++ ** y` and `++x ** y` all parse. The fuzzer still parenthesizes the base,
  so this is pinned by dedicated tests in `tests/es_parity.rs`.
- **Promises + async/await + event loop** — `new Promise`, `.then`/`.catch`/
  `.finally`, `Promise.resolve`/`reject`/`all`/`allSettled`/`race`/`any`; `async`
  functions/arrows/methods, `await`, rejection-as-throw; a host-driven loop
  draining `process.nextTick` → promise microtasks → timers
  (`setTimeout`/`setInterval`/`setImmediate`, `queueMicrotask`), Node ordering.

## Regular expressions — supported subset and known divergences

node-js translates the **overlapping** subset of JS regex that `fancy-regex` can
represent and rejects most of the rest at RegExp-construction time with a
`SyntaxError`.

**"It never silently mis-executes a pattern" is what this section used to say,
and it is not true.** Two counterexamples, both accepted and both producing the
wrong answer, measured on node v26.7.0:

| pattern | node v26.7.0 | node-js |
| --- | --- | --- |
| `new RegExp("(?i)abc")` | `SyntaxError: Invalid regular expression: /(?i)abc/: Invalid group` | compiles; `.test("ABC")` is `true` while `.ignoreCase` reports `false` |
| `/\1(a)/.test("xa")` | `true` — a JS forward reference matches the empty string | `false` |

`(?i)` is a Rust INLINE-FLAG group with no meaning in JS, which the reference
rejects outright; here it reaches the engine and changes matching behind a flag
reflector that denies it. The forward reference is the opposite shape: valid JS
that `fancy-regex` fails. The rejection rule below is still the rule for
everything the translator does not recognise — it is the boundary that is
imperfect, not the policy.

**Supported:** character classes (`[a-z]`, `[^0-9]`), the predefined classes
`\d \w \s \D \W \S` and word-boundary `\b`/`\B`, quantifiers (`* + ? {n} {n,}
{n,m}` + lazy `?`), anchors `^ $`, capturing/non-capturing/named groups
(`(...)`, `(?:...)`, `(?<name>...)`), alternation `|`, escapes, `\uXXXX`/`\u{...}`
(translated to Rust `\x{...}`), **backreferences** (`\1`, `\k<name>`) and
**lookahead / lookbehind** (`(?=)`, `(?!)`, `(?<=)`, `(?<!)`) — all provided by
`fancy-regex` 0.18 — and the flags `g` (global), `i` (ignoreCase), `m`
(multiline), `s` (dotAll), `u`, `y` (sticky), `d` (accepted; indices ignored).
`test`/`exec`/`match`/`matchAll`/`replace`/`replaceAll`/`split`/`search` and the
`$1`/`$&`/`` $` ``/`$'`/`$<name>`/`$$` replacement patterns + function replacers.

**Rejected (construction throws `SyntaxError`):** any pattern
`fancy-regex` cannot compile is rejected at RegExp-construction time
(`regexp.rs` maps the compile error to a JS `SyntaxError`) rather than silently
mis-executed. Backreferences and lookahead/lookbehind — previously listed here —
are **now supported** (see the Supported list above); verified against
`node v26.5.0`: `/(\w)\1/.test('aa')` → `true`, `/(?<=foo)bar/.test('foobar')`
→ `true`.

**Known behavioral divergences within the supported subset:**

- **Unicode class semantics.** Rust `regex` runs in Unicode mode, so `\d`/`\w`/
  `\s` match Unicode digit/word/space code points, whereas JS *without* the `u`
  flag matches only the ASCII sets. Identical on ASCII input (the fuzzer's
  `regex` mode uses ASCII inputs).
- **The match alphabet is code points, not code units.** `.index`, `lastIndex`
  and a replace callback's offset are now UTF-16 code-unit offsets and agree
  with node on astral input. What still differs is what a single-character
  pattern MATCHES: `fancy-regex` steps by Unicode scalar, JS (without the `u`
  flag) steps by code unit, so `"ab𝒳cd".match(/./g)` is 6 elements in node —
  the astral character split into its two surrogate halves — and 5 here. This
  is the regex engine's alphabet, a separate axis from string indexing, and it
  runs into the same lone-surrogate boundary documented above.
## FIXED — collection access was O(n^2), and the cause was local to node-js

Per-element access to a JS array, object, or `Buffer` used to cost quadratic
time. An earlier revision of this file blamed `fusevm::Value::Array` holding a
by-value `Vec<Value>`. **That attribution was wrong.** node-js never constructs a
`fusevm::Value::Array` at all — its arrays are `JsObj::Array(Vec<Value>)` in its
own heap, reached through a `Value::Obj(u32)` handle (`JsObj` in `src/host.rs`). The
fusevm `Arc`-backed array added in 0.19.0 therefore changed nothing here.

The real cause was four node-js sites that cloned an **entire heap cell just to
read its variant tag**, because `with_host` is a `RefCell` borrow and the code
inside each match arm re-enters the host, so a `&JsObj` borrow could not be held
across the match. `h.get(recv).cloned()` escaped the borrow — and deep-copied the
whole backing `Vec`/`IndexMap`/`String` every time:

| site | cost before |
| --- | --- |
| `get_property` (`src/builtins.rs`) | every property read copied the whole receiver, so `a[i]` and `a.length` were O(len) |
| `set_property` (`src/builtins.rs`) | up to five whole-receiver copies per assignment |
| `call_method` (`src/host.rs`) + `call_type_method` (`src/builtins.rs`) | every `a.push(x)` copied the whole array before dispatching |
| `buffer::byte_get` / `byte_set` (`src/stdlib/buffer.rs`) | one `buf[i]` materialised the whole buffer as a `Vec<u8>`; one `buf[i] = n` wrote every byte back |

The fix is not `Arc`/copy-on-write — JS arrays are mutable and aliased
(`const a=[1]; const b=a; b.push(2)` must be visible through `a`), and the heap
already gives correct aliasing because every handle points at one canonical cell.
The fix is to stop copying in order to *look*: `ObjKind` (`src/host.rs`) and
`JsHost::kind_of` (`src/host.rs`) return the discriminant alone, and `peek`
(`src/builtins.rs`) hands back only the one field an arm needs. `push`/
`unshift` take their return length from the same mutable borrow via `array_len`
(`src/builtins.rs`) instead of copying the array out to count it, and the
Buffer paths read and write the single element in place.

Summing every element of an array of `n` integers (`for-of` plus an indexed
loop), debug build, same machine — and an indexed `Buffer` read plus
`toString('hex')`:

| n | array before | array after | buffer before | buffer after |
| --- | --- | --- | --- | --- |
| 2,000 | 0.35 s | 0.02 s | 0.13 s | 0.03 s |
| 4,000 | 1.36 s | 0.04 s | 0.47 s | 0.05 s |
| 8,000 | 5.36 s | 0.09 s | 1.71 s | 0.11 s |
| 16,000 | 20.72 s | 0.18 s | 6.72 s | 0.22 s |
| 32,000 | 81.93 s | 0.36 s | 26.67 s | 0.45 s |

Both curves were quadratic (4x per doubling) and are now linear (2x per
doubling) — 228x at n=32,000 for arrays, 59x for Buffers. Object property access
was quadratic for the same reason and is linear too.

`express.json()` was never hit by this, and that remains true: `body-parser`
concatenates the socket chunks and hands a STRING to `JSON.parse`, so no
per-element JS loop runs over the body. Real express 5 POSTs of 25 KB → 207 KB
are flat in both builds (0.16–0.18 s before, 0.06–0.07 s after); the improvement
there is general property-access speedup, not a change in body-size scaling.

## FIXED — the process's own observables: exit status, exit events, raw stdout

Four things about how a program ENDS, and one about how it writes, none of
which any harness could report. `parity-scripts/run.sh` and `parity_fuzz` both
compared the exit status as zero-vs-nonzero, and `process.exitCode = 3` prints
nothing — so with stdout empty on both sides and the status collapsed to a
boolean, there was nothing left to compare. Both harnesses now compare the code
exactly (see the harness section in README).

- **`process.exitCode` was a decoration.** It stored and read back, and the
  process still exited 0. It is Node's accessor now, ported from
  `lib/internal/bootstrap/node.js`: a numeric string coerces (`"0x10"` exits 16,
  `"  "` exits 0), a non-numeric or empty string is `ERR_INVALID_ARG_TYPE`, a
  non-integer is `ERR_OUT_OF_RANGE`, and `null`/`undefined` clear the slot.
  `process.exit()` with no argument uses it.
- **`process.on('exit')` and `process.on('beforeExit')` never fired at all.**
  Both run now: `beforeExit` when the loop drains on its own (again if a
  listener schedules more work), `exit` exactly once — on the normal path, on
  `process.exit()`, and on an uncaught exception. A code an `exit` listener
  assigns wins; an uncaught exception forces 1 first, as Node does.
- **`process.once` was an alias of `process.on`.** The listener fired on every
  emit and stayed in `process.listeners()`.
- **`process.stdout.write` put every chunk through `ToString`.**
  `write(Buffer.from([0xff,0xfe,0x41]))` printed the 15 bytes of
  `[object Object]`; `write('4142','hex')` printed the four characters of the
  literal instead of the two bytes `AB`. Byte views go out untouched now and a
  string chunk is decoded with its encoding, with Node's `ERR_INVALID_ARG_TYPE`
  for anything else. The host's capture buffer became `Vec<u8>` so the byte path
  is real rather than a `String` round trip that would reintroduce `U+FFFD`.
- **`-p`/`--print` did not exist** — `node -p '1+1'` was
  `error: unexpected argument '-p' found`.

Two corpus assertions that were green throughout are worth recording, because
each was shaped so it could not fail: `parity-scripts/lang/26_process_exit.js`
registered `process.on("exit", () => {})` with an EMPTY body, which cannot tell
"fires" from "never fires"; and `parity-scripts/stdlib/20_process.js` asserted
`typeof process.exitCode === "undefined" || … === "number"`, a disjunction
satisfied whether or not the property does anything. Both are strengthened, and
neither original assertion was removed.

## FIXED — object-model gaps reachable from a one-line `-e`

- **`Error.prototype.toString` did not exist.** `Object.prototype.toString`
  inherited through to errors instead — and merely READING `Error.prototype` or
  `Object.prototype` materialises that thunk. `x instanceof Error` performs that
  read, so ordinary code flipped `String(err)` from `Error: m` to
  `[object Error]` for the rest of the process, retroactively, including errors
  created earlier.
- **`Error.prototype` read as a THUNK, not the object errors link to.** `typeof
  Error.prototype` was `"function"`, and
  `Object.getPrototypeOf(new Error("x")) === Error.prototype` was false. Fixing
  it also fixed `Object.getPrototypeOf(TypeError.prototype) === Error.prototype`
  and `Object.getPrototypeOf(new TypeError("x")) === TypeError.prototype`.
- **`/` never ran `ToPrimitive`.** It is a builtin rather than a native op
  (fusevm's `Op::Div` disagrees with JS on a zero divisor), so it bypassed the
  numeric hook that `+ - * % **` go through: `({valueOf(){return 7}}) / 2` was
  `NaN` where `* 2` was `14`, and `new Date(2) / 1` was `NaN` instead of `2`.
- **A computed object-literal key skipped `ToPropertyKey`.** `{ [obj]: 1 }` keyed
  on `"[object Object]"` while `a[obj] = 1` keyed on the `toString` result.
- **`Symbol.keyFor` returned the DESCRIPTION**, so every symbol looked
  registered: `Symbol.keyFor(Symbol("k"))` was `"k"` rather than `undefined`.
- **`__proto__` answered only for plain objects and only from an explicit
  link.** `[].__proto__` was `undefined` and `({}).__proto__` was `null`, while
  `Object.getPrototypeOf` was right for both. The setter re-linked
  unconditionally, so `o.__proto__ = 5` set the prototype to the number 5; it
  takes only an Object or `null` now, and on a null-prototype object (which
  inherits no such accessor) the assignment is an ordinary own-property write.
- **`RegExp.prototype.flags` returned the literal's spelling** rather than the
  spec's canonical order — `/a/gid.flags` was `"gid"`, node says `"dgi"` — and
  `hasIndices`/`unicodeSets` were absent.
- **`Boolean.prototype` had no methods.** A boolean is not a heap object here, so
  `true.toString()` threw `is not a function`.
- **`BigInt(1e21)` threw** (`Number.prototype.toString` goes exponential at
  1e21 and `parse_bytes` cannot read `"1e+21"`), and
  **`(1e21).toLocaleString()` printed `1e,+21`** — the thousands separator
  applied to the exponent. Note the two take deliberately DIFFERENT sources:
  `BigInt` uses the f64's exact decimal expansion, so `BigInt(1e30)` is
  `1000000000000000019884624838656n` as in node, while `toLocaleString` expands
  the SHORTEST repr, so `(1e100).toLocaleString()` is 1 followed by a hundred
  zeros. Both measured, neither assumed.

## FIXED — the locale surface, and why it is machine-independent

Most of `toLocale*` was missing outright: `String.prototype.toLocaleLowerCase`/
`toLocaleUpperCase`/`toLocaleString`, `Object.prototype.toLocaleString`,
`Array.prototype.toLocaleString`, and all three `Date.prototype.toLocale*` forms
threw `is not a function`, and `BigInt.prototype.toLocaleString` skipped
thousands grouping. All are implemented, in the fixed en-US shape this runtime
already used for `Number.prototype.toLocaleString`, at UTC, with the
`locales`/`options` arguments accepted and ignored.

`String.prototype.normalize` remains the identity (no normalization tables) but
now VALIDATES the form: node throws `RangeError` outside NFC/NFD/NFKC/NFKD, and
a try/catch support probe used to be told every form worked.

**node-js reads no `LANG`, `LC_ALL` or `TZ` anywhere**, and that is a property
worth stating rather than a coincidence: `Date` is hardwired to UTC, the number
formats to en-US, `normalize` to the identity, and `toUpperCase`/`toLowerCase`
to Rust's locale-INDEPENDENT Default Case Conversion, which is what the spec
mandates for the non-`Locale` forms. So node-js's output is byte-identical on
every machine. Reference `node` is NOT — `(1234.5).toLocaleString()` is
`1.234,5` under `de_DE`, `'ä'.localeCompare('z')` is `1` under `sv_SE` and `-1`
under `de_DE`, and `new Date(0).getHours()` is `9` under `Asia/Tokyo`. All three
harnesses therefore PIN `TZ=UTC LANG=LC_ALL=en_US.UTF-8` rather than inheriting
the developer's, so a corpus case touching the locale surface cannot pass on one
machine and fail on another.

`localeCompare` is unchanged and still the ASCII approximation documented above:
it diverges from ICU for any non-ASCII input (`'ä'.localeCompare('z')` is `1`
here, `-1` in node), ignores the `locales` and `options` arguments, and does not
raise node's `RangeError: Invalid language tag` for a malformed tag.

## FIXED in round 6 — the reference wordings that were frozen in the source

Round 6 audited every string literal in `src/` that purports to be node's own
words. Each row was measured against node v26.7.0 before and after.

**The round's theme, found here exactly once.** `new URL("/x")` threw
`TypeError: Invalid URL: /x` from `src/stdlib/url.rs` while the sibling
`url_legacy::invalid_url` was already emitting the current
`TypeError [ERR_INVALID_URL]: Invalid URL`. One code path had been updated and
the other had not, and no version gate can see a string frozen in the source.

**Fabricated — text no node ever printed:**

| site | was | node v26.7.0 |
| --- | --- | --- |
| `process.chdir("/nope")` | `ENOENT: No such file or directory (os error 2), chdir '/nope'` (Rust's `io::Error` Display) | `ENOENT: no such file or directory, chdir '<cwd>' -> '/nope'` |
| `crypto.createHash("nope")` | `Digest method not supported: nope` | `Digest method not supported` |
| `[...5]` | `number is not iterable` (the `typeof`) | `5 is not iterable` (the VALUE) |
| `FinalizationRegistry#register(1,2)` | `…register: target must be an object` | `…register: invalid target` |
| `FinalizationRegistry#unregister(1)` | `…unregister: unregister token must be an object` | `Invalid unregisterToken ('1')` |
| `util.styleText("nope","x")` | `must be a valid util.inspect.colors key` | enumerates every accepted name |
| `stream.pipeline()` | `pipeline requires at least one stream` | `The "streams[stream.length - 1]" property must be of type function. Received undefined` |

`styleText`'s list is now GENERATED from the same `(name, open, close)` table the
lookup reads, so a style cannot be accepted-but-unlisted, and the count is never
written down.

**Stale — a real wording that had drifted from its sibling.**
`BigInt.prototype.toString(37)` said `radix must be between 2 and 36` where V8
says `radix argument must be`. Both `toString` sites now share one constant —
and `Number.prototype.toString` gained the validation it never had at all: an
out-of-range radix silently fell back to base 10, so `(1).toString(37)` returned
`"1"` and a support probe was told every radix worked.

**Missing `code` — the message matched, `err.code` was `undefined`** at
`new URL`, `fileURLToPath`, `require()` (`MODULE_NOT_FOUND`),
`validateHeaderName`/`validateHeaderValue` (both forms), `buf.swap16`,
`timingSafeEqual`, `randomInt`, `process.exit` (both forms) and `process.kill`.

Node does NOT bracket the code uniformly, and the difference is observable:

| | `String(err)` | `err.code` |
| --- | --- | --- |
| JS layer (`process.exit(1.5)`) | `RangeError [ERR_OUT_OF_RANGE]: …` | set |
| native layer (`new URL("/x")`) | `TypeError: Invalid URL` | set |

So there are two constructors (`host::coded_error` and
`host::plain_coded_error`); one head form would have to pick one and be wrong
about the other.

**Deliberate divergences, deliberately left alone:** the terse `node: <reason>`
where node prints a V8 stack, `localeCompare`'s ASCII approximation,
`normalize` as the identity, and a lone surrogate rendering as `U+FFFD`.

## FIXED in round 6 — Rust names that look like the reference's

| lookalike | symptom |
| --- | --- |
| `(x + 0.5).floor()` as `Math.round` | `Math.round(0.49999999999999994)` was 1 (must be 0); `Math.round(4503599627370497)` perturbed an exact integer |
| `f64::powf` as `Math.pow`/`**` | IEEE `pow` makes `(-1) ** Infinity` and `1 ** NaN` equal 1; the spec says NaN |
| `f64 as i64 as u32` as `ToInt32` | Rust SATURATES an out-of-range float cast, so `1e300 | 0` was -1 and `1e300 >>> 0` was 4294967295 — both are 0 in every engine |
| `>` / `<` for `Math.max`/`min` | cannot separate +0 from -0, so `Math.max(-0, 0)` kept the -0 |
| `char::is_whitespace` as ECMA `WhiteSpace` | differs in BOTH directions: `U+FEFF` is JS whitespace and not Unicode `White_Space`; `U+0085` is the reverse |
| `i64::from_str_radix` in `parseInt` | overflowed past ~19 digits into `NaN` |
| longest plausible run in `parseFloat` | is not the longest VALID prefix: `parseFloat("1e")` was `NaN`, not 1 |

Nine `Math` members did not exist at all — `imul`, `log1p`, `expm1` and the six
hyperbolics; `typeof Math.imul` was `undefined`. Their results are within 1 ULP
of node over a 2178-point sweep. That residue is macOS libm against V8's fdlibm
and is not specific to the new members: it also affects `tan` (39 points),
`atan` (19) and `cbrt` (12), which this round did not touch.

`**` moved off fusevm's native `Op::Pow` onto a builtin, so `cache::SCHEMA` went
7 → 8; a v7 blob still carries `Op::Pow` and would replay the IEEE answer from
cache. The `Math.*` additions needed no bump, and that is measured rather than
assumed: `--dump-bytecode` is byte-identical for a known and an unknown `Math`
method name, because the name is a constant and dispatch happens at run time.

## FIXED in round 6 — identity, and the assertions that could not see it

`process.env`, `process.argv`, `process.execArgv`, `process.versions` and the
three std streams were rebuilt on every read. `process.env === process.env` was
`false`, and `process.env.NODE_ENV = "production"` wrote to a throwaway object
and read back `undefined`. They are memoized for the host's lifetime now.

`globalThis` was an empty bag: `globalThis.process`, `.console`, `.Math` and
`.JSON` were all `undefined`, so `process === globalThis.process` was `false`
and any `globalThis.X` feature probe reported the feature missing. A read off
the global object now falls through to the same lazy binding the bare
identifier gets, and `globalThis.x = 1` creates a real global binding (it used
to set an own property that the bare `x` could not see). The CommonJS wrapper
locals (`require`, `module`, `exports`, `__filename`, `__dirname`) stay OFF the
global object, as in node.

None of that was visible to the corpus, because `parity-scripts/stdlib/20_process.js`
checked the whole surface with `typeof x === "string"` — satisfied by any
string. Those lines are kept and each is now paired with a check that re-derives
the same read from an independent source (`process.argv[0] === process.execPath`,
`process.version.slice(1) === process.versions.node`) or compares identity.

The same sweep found `typeof process.hrtime.bigint === "function"` answering
`true` while CALLING it threw `bigint is not a function` — an existence check
that a missing implementation satisfies. `hrtime.bigint` is implemented, and the
corpus now calls it.

A `buf.readXxx(offset)` past the end returned 0 rather than throwing, which is
indistinguishable from a buffer that really holds a zero there. All six
fixed-width reads validate now (`ERR_OUT_OF_RANGE`, or
`ERR_BUFFER_OUT_OF_BOUNDS` when the buffer could never hold the value), and
`parity-scripts/stdlib/17_buffer_statics.js` exercises each failure mode instead
of only the truthiness conjunction `safe-buffer` evaluates.

`tests/embed.rs::output_before_a_throw_is_kept` asserted `result.is_err()`,
which a parse failure or an unbound name satisfies just as well as the `throw`
under test. It pins the error text now.

## FIXED in round 6 — lone surrogates in a string literal

A `\uD800` escape was DROPPED by the lexer rather than substituted, so
`"\ud800".length` was 0 where this runtime's own documented `U+FFFD` policy
requires 1 — the policy exists precisely to keep the code-unit arithmetic exact,
and dropping the unit broke the invariant instead of implementing it.

An in-literal surrogate PAIR is also rejoined now: `"\ud83d\ude00"` is the
ordinary ASCII-safe way to write an astral character, and decoding each half
independently turned every such literal into two `U+FFFD`s. Two SEPARATE
literals still cannot rejoin (`"\ud83d" + "\ude00"`), which is the documented
Rust-`String` boundary above, not a new gap.

## Still open — found by the round-5 doc audit, not yet fixed

Each of these was verified against node v26.7.0 and is a real divergence; none
is claimed fixed anywhere in this file.

| gap | node v26.7.0 | node-js |
| --- | --- | --- |
| `DataView` | `function` | `undefined` — the constructor does not exist |
| `arguments.callee` | the running function | `undefined` |
| a tagged template's object and its `raw` | frozen, elements non-writable | mutable, elements writable (the DESCRIPTOR triple is already correct) |
| a class body's own binding for the class name | immutable — `class E { static z = (E = 1) }` throws | assignment succeeds |
| `/(?i)abc/` | `SyntaxError: Invalid group` | accepted; matches case-insensitively while `.ignoreCase` reports `false` |
| `/\1(a)/` (a forward reference) | `true` — matches the empty string | `false` |
| `/\cA/` (a control escape), `/\052/` (an octal escape) | valid patterns, both match | `SyntaxError` — over-rejected |
| `Buffer.alloc(-1)`, `Buffer.alloc('x')`, `path.join(1)` | throw a coded error | do not throw (the fixed-width `buf.readXxx` reads DO throw as of round 6) |
| `Buffer.alloc(2**40)` | returns promptly (the allocation is lazy) | hangs — killed at 8s, materialising a 1 TiB byte vector |
| `url.resolve` with an uppercase scheme, an empty port, or a Unicode host | lowercases / strips / punycodes | leaves the input as-is |
| `Object.getPrototypeOf(class B extends A {})` | `A` | not `A` (static-method LOOKUP still works) |

## Partial / simplified semantics (runs, but not byte-identical to node in edge
cases the fuzzer is scoped away from)

- **`util.inspect` `compact` depth-gate (only under a raised `{depth}`).** Node
  forces an object/array onto multiple lines when its deepest descendant is `≥
  compact` (3) levels below it, even if the single-line form fits `breakLength`.
  Only the `breakLength` 80 fit is modelled, not the depth-gate, so
  `util.inspect(x, {depth: N>2})` on a `≥4`-level-deep structure may stay on one
  line where Node breaks it. `console.log` (fixed `depth: 2`, where the gate can
  never fire) is byte-identical.

- **Array holes.** A node-js array is a dense `Vec<Value>`, so an elision or a
  `delete` leaves `undefined` where V8 leaves a HOLE. Everything that
  distinguishes the two therefore differs: `[1,,3].forEach` runs 3 times
  (Node: 2), `1 in [1,,3]` is `true` (Node: `false`), and `console.log([1,,3])`
  prints `[ 1, undefined, 3 ]` (Node: `[ 1, <1 empty item> ]`-style). `a[3]=1`
  on an empty array materialises three `undefined`s rather than three holes.
  Representing holes needs a sentinel through every array path (length, index
  read/write, every iteration method, `inspect`), which is an array-model change
  rather than an addition, so it is listed rather than half-done.

- **The `arguments` object is a real Array.** `Array.isArray(arguments)` is
  `true` and `Object.prototype.toString.call(arguments)` is `[object Array]`
  (Node: `false` / `[object Arguments]`). Those two reads are the whole
  divergence: `.length`, indexing, `Array.prototype.slice.call(arguments)`,
  spread, `for…of`, and an arrow's lexical capture of the enclosing function's
  `arguments` all match, because an Array answers all of it. A real Arguments
  exotic needs its own `ObjKind` and a `[[ParameterMap]]`.

- **The ENTRY script is not wrapped in the CommonJS wrapper.** A `require`d
  module is (`module.rs:315`), and there `__filename`/`__dirname`/`module`/
  `exports`/`arguments` all match Node down to `arguments.length === 5`. In the
  file passed on the command line they are `undefined`, and top-level
  `arguments` is a `ReferenceError`. Node runs both through the same wrapper.

- **`String.prototype.normalize` is the identity.** `'é'.normalize('NFC')`
  returns the input unchanged, so a decomposed string keeps its length (2, not
  1). Real NFC/NFD/NFKC/NFKD needs Unicode normalization tables, which node-js
  does not vendor.

- **Strict mode is not tracked, so a frozen write never throws.** `'use strict';
  Object.freeze(o); o.a = 2` silently does nothing here; Node throws
  `TypeError: Cannot assign to read only property`. The sloppy-mode outcome (the
  write is ignored, `delete` reports `false`) is correct and is what the `freeze`
  fuzzer mode pins.

- **`util.inspect` does not see a `Symbol.toStringTag` GETTER.** An inherited
  DATA property renders as the `Ctor [Tag] ` prefix, but
  `class C { get [Symbol.toStringTag]() { return 'Cee' } }` prints `C { … }`
  where Node prints `C [Cee] { … }`. `inspect` runs under the host's `RefCell`
  borrow and invoking the getter would re-enter the VM.
  `Object.prototype.toString.call(x)` DOES run the getter and reports
  `[object Cee]` — that path is not inside the borrow.

- **A `Timeout`/`Immediate` handle has no reachable prototype object and no own
  `Symbol.toPrimitive`.** The handle's methods dispatch through the native
  `instance_call` table rather than sitting on a real prototype, so
  `Object.getOwnPropertyNames(Object.getPrototypeOf(t))` is empty where Node
  lists `constructor,refresh,unref,ref,hasRef,close`. Calling the methods,
  `t.constructor.name`, and coercion (`String(t)`/`Number(t)` → the timer id,
  via `valueOf`/`toString` rather than Node's own `Symbol.toPrimitive`) all
  behave correctly; only reflection over the prototype differs. This is the
  general native-instance shape, not a timer-specific one — `URL` reports the
  same empty prototype, while `Buffer` has a real linked one.

- **`ref`/`unref` on a socket or server do not change loop liveness.**
  `socket.ref`/`unref` are accepted no-ops, and `server.ref`/`unref` are absent
  entirely (`server.unref()` throws `TypeError: server.unref is not a
  function`), so a program that unrefs its listener keeps the process alive
  where Node exits. Timers do honor the handle bit; sockets and servers do not.
  Fixing this needs more than adding the methods: `open_handles` is a bare
  counter with no per-object identity, so an `unref` cannot tell whether the
  handle it would decrement is still open, and an `unref` after a `close` would
  consume some *other* handle's count and drop the loop early — turning a
  hang into the worse failure of a live server exiting silently. It needs
  per-handle identity (each socket/server owning a token it can release at most
  once), which is a change to the handle model rather than to these methods.

## Fixed since the initial parity sweep (previously divergences, now correct)

Recorded so the same gaps are not "re-discovered" as regressions. All verified
against `node v26.5.0`:

- **Number → string exponential threshold.** `(1e21).toString() === "1e+21"`,
  `(1e-7).toString() === "1e-7"` per the ECMAScript Number::toString layout.
- **`x / 0` division.** `1/0 === Infinity`, `0/0 === NaN` (fusevm's native `Op::Div`
  returns `Undef` for a zero divisor, so `/` lowers to a node-js builtin).
- **`+` operand coercion (`ToPrimitive`).** `[1,2,3] + 3 === "1,2,33"`,
  `{} + [] === "[object Object]"`; a user `toString`/`valueOf` is now invoked by
  `String(x)` / template interpolation / object keys.
- **`==` loose equality.** Abstract Equality with `ToPrimitive`.
- **`Number.prototype.toFixed`/`toPrecision`/`toExponential`** — round half away
  from zero on the exact value, preserve the sign of a zero result, keep full
  precision at large magnitudes, and throw the spec `RangeError` outside the
  0..100 / 1..100 argument ranges.
- **`JSON.stringify` honors `toJSON`** — user methods, class methods, and the
  native `Date`/`Buffer`/`URL` accessors, applied before serialization. A
  self-referential value throws `TypeError: Converting circular structure to
  JSON` instead of overflowing the stack.
- **Optional call `f?.()`** — short-circuits to `undefined` on a nullish callee
  without evaluating the arguments, keeping the receiver for `obj.m?.()`.
- **Named function expressions** — `const f = function fact(n) { … fact(n-1) … }`
  binds its own name inside the body.
- **Private brand checks** — `#field in obj`.
- **Thrown-error identity across coroutines** — a `throw` inside an `async`
  function or generator body reaches `.catch`/`catch` as the ORIGINAL error
  object (it used to be rebuilt from a string, losing the subclass).
- **`Math.hypot`** — scaled algorithm matching V8's last-ULP result.
- **`Math.round`** preserves negative zero.
- **`String.prototype.slice`/`substr`** — reversed bounds yield the empty string;
  `substr` handles a negative start.
- **`parseFloat`** parses `Infinity` / `-Infinity`.
- **`Number.prototype.toString(radix)` for a non-integer receiver.** Fractional
  digits are now emitted in the target radix (V8's `DoubleToRadixCString` port,
  round-half-to-even with ULP-sized termination): `(3.5).toString(2) === "11.1"`,
  `(255.5).toString(16) === "ff.8"`; integer receivers unaffected.
- **`Object.create(null)` under `instanceof Object`.** An explicit null-prototype
  object (via `Object.create(null)` or `Object.setPrototypeOf(o, null)`) is now
  tracked distinctly from a bare `{}`, so `Object.create(null) instanceof Object`
  is `false` while `({}) instanceof Object` stays `true`.
- **ES2023 change-by-copy Array methods.** `toSorted`/`toReversed`/`toSpliced`/
  `with` return a new array leaving the receiver unchanged; `with` throws
  `RangeError: Invalid index : <i>` on an out-of-range index.
- **Integer-key property ordering (`OrdinaryOwnPropertyKeys`).** Own array-index
  keys now enumerate in ascending numeric order before insertion-ordered string
  keys, consistently across `Object.keys`/`values`/`entries`,
  `getOwnPropertyNames`, `for-in`, spread `{...o}`, `Object.assign`, `JSON.parse`
  results, and `JSON.stringify`. `2^32-1` and non-canonical numeric strings
  (leading zero, sign, fraction) stay string keys.
- **Object `console.log` multiline `breakLength` wrapping.** A single-line object
  wider than 80 columns now wraps one property per line like arrays already did,
  including the constructor / `[Object: null prototype]` tag in the width
  calculation. `>6`-element arrays nested inside objects also render at the
  correct indentation.
- **`instanceof` for native-tagged instances.** A `WeakRef`, `FinalizationRegistry`,
  `TextEncoder`, `TextDecoder`, etc. is now an instance of the builtin whose name
  matches its hidden `@@native` tag (`new WeakRef({}) instanceof WeakRef` is
  `true`), and `Object.prototype.toString.call(x)` brands each of them
  (`[object WeakRef]`, `[object URL]`, `[object URLSearchParams]`, …). Only the
  `@@native` tags that carry a real `Symbol.toStringTag` in Node are listed —
  `EventEmitter`, `Server`, `Hash`, `Readable` and friends are plain classes
  with no tag, so they stay `[object Object]` as they do in Node.
- **`FinalizationRegistry`** — constructor requires a callable; `register(target,
  held[, token])` and `unregister(token)` enforce their `TypeError`s and
  `unregister` returns the correct boolean. Cleanup callbacks never fire because
  the heap holds every value strongly (a spec-permitted approximation, same basis
  as `WeakRef`).
- **`ToPrimitive` (7.1.1) is real, so a user `valueOf` actually runs.** Every
  conversion site — `+`, `- * / % **`, the relational operators, `==` against a
  primitive, `ToNumber` (unary `+`/`~`, `Number(x)`), the bitwise operators,
  `ToPropertyKey` (`obj[keyObject]`) and `Array.prototype.join`/`toString` —
  converts an object through `ToPrimitive` with the right hint before doing
  anything else. It used to read the raw internal string form instead, so
  `{valueOf(){return 7}} + 1` was `"[object Object]1"`, `+obj` was `NaN`, and
  `obj == 7` was `false`.
  - `Symbol.toPrimitive` is honored and takes precedence over
    `valueOf`/`toString`.
  - `Date` overrides the DEFAULT hint to `"string"` (21.4.4.45), so `date + 1`
    concatenates while `date - 0` is arithmetic. `Date.prototype.valueOf` also
    reaches the Date dispatcher now: it was shadowed by
    `Object.prototype.valueOf` and returned the Date object, which is why `+d`
    and `d - 0` were `NaN`.
  - An object with no reachable `toString`/`valueOf` (`Object.create(null)`)
    throws the spec `TypeError: Cannot convert object to primitive value`
    instead of silently producing `"[object Object]"`; `toString`/`valueOf` on
    a null-prototype object read as `undefined`.
  - The conversion has to happen OUTSIDE the host's `RefCell` borrow (calling a
    JS `valueOf` re-enters the VM), so it lives in `host::to_primitive` /
    `to_number_value` / `to_property_key` and the ops call those before entering
    `JsHost::arith`.
- **Well-known symbols and symbol-keyed properties.** `Symbol.toPrimitive` and
  `Symbol.toStringTag` exist alongside `Symbol.iterator`/`asyncIterator` — and
  only those four, because those are the ones node-js acts on (a
  `Symbol.hasInstance` that read as a symbol while `instanceof` ignored it would
  be a silent fake). Their description is now the ECMAScript name, so
  `String(Symbol.iterator)` is `Symbol(Symbol.iterator)`; identity is tracked by
  symbol id, so `Symbol.for('Symbol.iterator')` and `Symbol('Symbol.iterator')`
  stay distinct from the well-known one.
  - `Symbol.toStringTag` (own or inherited, data property or getter) replaces
    the builtin brand in `Object.prototype.toString`, and `function*` /
    `async function` / `async function*` / a suspended (async) generator /
    `Math` / `JSON` / `Reflect` all brand as themselves.
  - `Object.getOwnPropertySymbols` exists, `Reflect.ownKeys` lists the symbol
    keys after the string keys, and object spread / `Object.assign` copy own
    enumerable symbol-keyed properties (`CopyDataProperties`, 7.3.25) — they
    used to drop them. `Object.keys`/`for-in`/`JSON.stringify` still skip them,
    as they must.
- **An arrow function no longer shadows the enclosing `arguments`.**
  `FunctionDeclarationInstantiation` (10.2.11) creates the `arguments` binding
  only for a non-arrow function, so `arguments` inside an arrow resolves
  lexically. node-js bound a fresh empty one in every call frame, which made
  `function f() { const g = () => [...arguments]; return g(); }` see zero
  arguments (`host.rs`, `bind_params`).
- **`util.inspect` symbol keys, tag prefix, constructor prefix and quoting.**
  An own enumerable symbol-keyed property renders as `Symbol(desc): value`; an
  INHERITED `Symbol.toStringTag` renders as the `Ctor [Tag] ` prefix (an own
  enumerable one does not, since it is already listed as a property — V8 does
  the same to avoid printing it twice). The constructor prefix now also covers
  `function F(){}` instances (`F { y: 2 }`), found by walking the prototype
  chain for an own `constructor` the way V8's `getConstructorName` does. String
  quoting is a port of Node's `strEscape`: single quotes normally, double when
  the string contains a `'` but no `"`, a backtick when it contains both, and
  the C0 controls escape through Node's `meta` table (`\n`/`\t`/`\b`/`\f`/`\r`
  short forms, `\x0B`/`\x00`/`\x7F` uppercase hex otherwise).

- **Timers keep the process alive (`ref`/`unref` handle counting).** Verified
  against `node v26.7.0`. `setInterval` used to fire exactly ONCE and then let
  the loop drain, so `setInterval(fn, 1000)` exited immediately where real node
  runs until killed — a poller or heartbeat silently did nothing instead of
  failing loudly. An interval now re-arms itself (before invoking its callback,
  so a `clearInterval` from *inside* the callback still cancels it) and, being a
  referenced handle, holds the loop open exactly as in Node. The loop stays
  alive while a microtask, an open handle, or a *referenced* timer is pending;
  an unref'd timer still fires while something else holds the loop open but
  never holds it by itself. A pending interval also forces the real-clock
  regime — on the virtual clock, where time never advances, it would re-fire at
  the same instant forever and starve every longer-delay timer behind it.
  `setTimeout`/`setInterval` now return a `Timeout` and `setImmediate` an
  `Immediate`, carrying `ref`/`unref`/`hasRef`/`refresh`/`close`; both coerce to
  their integer timer id, so code that stored the previously returned number
  still clears correctly. The absent `clearImmediate` global was added.

- **An array's and a function's non-index own properties are real own
  properties.** Verified against `node v26.7.0`. Those keys (`arr.foo`,
  `arr[sym]`, a `str.match()` result's `index`/`input`/`groups`, anything
  assigned to a function) live in node-js's fn-prop side table rather than in an
  object property map, and every reflection path read only the property map — so
  `Object.keys`, `Object.entries`, `Object.getOwnPropertyNames`,
  `Object.getOwnPropertyDescriptor(s)`, `Reflect.ownKeys`,
  `Object.getOwnPropertySymbols`, object spread, `Object.assign`, `for-in`, the
  `in` operator and `delete` all behaved as though the property did not exist,
  and `console.log` dropped it. All of them now read the side table.
  - `Object.getOwnPropertyNames(arr)` reports the indices, then the exotic
    `length`, then the string keys — `OrdinaryOwnPropertyKeys` order, which is
    also where `length` sits in Node (it used to be appended last).
  - An array's `length` is now the array exotic's own property (10.4.2):
    writable, non-enumerable, non-configurable, so `delete a.length` reports
    `false` and its descriptor matches Node's.
  - A function's exotic `length`/`name`/`prototype` and a class's methods are
    non-enumerable, so `Object.keys(fn)` is exactly what a script assigned while
    `getOwnPropertyNames(fn)` reports `length,name,prototype,…` as V8 orders it.
  - `util.inspect` renders both halves: `[ 1, foo: 'bar', Symbol(k): 5 ]` for an
    array, `[Function: f] { z: 1 }` and `[class C] { x: 1 }` for a callable. An
    anonymous function expression also inspects under its INFERRED name
    (`const f = function(){}` → `[Function: f]`); inspect read the raw FuncDef
    name and printed `[Function (anonymous)]`.

- **`delete o[k]` keyed through `String(k)` instead of ToPropertyKey.**
  `delete o[Symbol('k')]` deleted a property literally named `"Symbol(k)"` and
  left the symbol-keyed one in place; an object with a `Symbol.toPrimitive` /
  `valueOf` key never reached its conversion. The read and the write already
  went through ToPropertyKey (7.1.19); `delete` now does too, and
  `Reflect.deleteProperty` shares the same `[[Delete]]` implementation instead
  of its own array-blind copy.

- **Only a constructor owns a `prototype` property.** MakeConstructor (10.2.5)
  runs for an ordinary function definition and for every generator; an arrow, a
  MethodDefinition (`{ m(){} }`, a class method/accessor) and an async function
  are not constructors. node-js materialised a `prototype` object on first read
  for ANY function, so `({m(){}}).m.prototype` was an object where Node has
  `undefined`. A new `FuncDef.is_method` flag (set by the parser for object
  methods/accessors and by the compiler for class members) drives it, and the
  bytecode-cache SCHEMA is bumped so no v5 blob replays the old shape.

- **A class body installs its methods before its static fields.**
  ClassDefinitionEvaluation (15.7.14) evaluates methods and accessors with the
  class body and runs the static-field initializers afterwards (step 32).
  node-js emitted members in source order, so `class C { static x = 1; static
  m(){} }` reported own keys `x,m` where Node reports `m,x`.

- **A match result's `groups` is a null-prototype object.** 22.2.7.2 builds it
  with `OrdinaryObjectCreate(null)`, so `m.groups instanceof Object` is `false`
  and `console.log(m)` tags it `[Object: null prototype] { … }`. It was an
  ordinary object.

- **A tagged template's `raw` is a frozen, non-enumerable own property.**
  GetTemplateObject (13.2.8.4) defines it non-writable, non-enumerable and
  non-configurable. It was an ordinary enumerable property, which put it in
  `Object.keys(strings)`.

- **A builtin namespace enumerated its keys but not their values.**
  `own_enum_entries` reads a property map, and a namespace (`require('path')`,
  `Buffer`) has none — its members are resolved on demand by
  `builtins::namespace_property`, which re-enters the host and so cannot run
  under that borrow. Every value therefore came back `undefined` while the keys
  came back right, so `{...require('path')}.join` and
  `Object.assign({}, require('buffer')).Buffer` were `undefined` where node
  v26.7.0 gives functions. `Object.keys`/`values`/`entries` were unaffected
  (they resolve through `builtins`, not through the borrow), which is why the
  gap survived: two enumeration paths, only one of them value-less.
  `host::own_enum_entries_deep` now resolves a namespace's values the same way a
  property read does.

- **`require('stream/web')` enumerated no keys at all.** `stdlib::resolve` maps
  the specifier onto the `stream/web` namespace, but `namespace_keys` is built
  from `namespace_methods` ∪ `namespace_ctors` and the module exports nothing
  but classes (`stream_web::METHODS` is empty) — so neither table backed it.
  `Object.keys(require('stream/web')).length` was 0 against node v26.7.0's 18,
  even though `require('stream/web').ReadableStream` resolved fine.
  `namespace_ctors` now returns `stream_web::CLASSES`, giving the 17 classes
  node-js implements (node's 18th, `ReadableStreamTee`, is not implemented, and
  `namespace_keys` deliberately advertises the working set rather than node's
  full export list).

- **NamedEvaluation runs at every site the grammar calls for it.** 10.2.9
  SetFunctionName used to fire only for a `const`/`let`/`var` declarator and a
  named function expression, so a function defined anywhere else kept `.name ===
  ""`. It now also fires for assignment to an identifier (`h = function(){}`),
  every object property definition — `key: value`, concise method, and
  `get`/`set` accessor, which carry the `get `/`set ` prefix — class fields
  static and instance, class accessors, parameter defaults and destructuring
  defaults. A COMPUTED key resolves at run time through the new `NAMED_EVAL`
  builtin, so `{ [k]: () => {} }` is named after the key's value and a symbol
  key gives `[description]`. It is applied from the SYNTAX, never from the
  value: `const anon = (0, function(){}); ({ m: anon }).m.name` stays `""` in
  node v26.7.0, and renaming by value would rewrite `.name` on a function the
  program still holds under another binding.

  Two sites remain unnamed. A computed ACCESSOR key (`{ get [k](){} }`) is the
  one member position whose key is no longer reachable on the stack when the
  function is pushed — `kind` sits between them — so it keeps the empty name.
  And logical assignment (`h ||= function(){}`, which node names `h`) is parsed
  by desugaring to `h = h || function(){}`, a form node deliberately leaves
  unnamed; the two are indistinguishable in the AST, so naming it would trade
  one divergence for another.

- **A class name is bound inside its own body.** 15.7.14 steps 8-17 evaluate a
  class body in its own environment holding an immutable binding for the class
  name, initialized to the class itself before the static-field initializers of
  step 32. node-js had no such environment: the only binding was the outer one a
  class DECLARATION installs after the body has already run, and a class
  EXPRESSION never gets one at all, so `class C { static x = C.m(); static
  m(){return 5} }` and `const K = class Inner { static self = Inner.name }` both
  threw `ReferenceError`. The binding does not leak outward — `typeof Inner` is
  still `undefined` outside the expression.
