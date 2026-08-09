# node-js — known gaps and unimplemented behavior

## Node core-module coverage
Implemented natively (verified vs node v26): `assert`(+`/strict`), `buffer`,
`child_process` (exec/spawnSync/execSync; `spawn` is sync-backed, not a live
streaming ChildProcess), `console`, `crypto` (hashes/hmac), `dns` (lookup/resolve
via std), `diagnostics_channel`, `events`, `fs`, `http`, `net`, `os`, `path`
(+`/posix` +`/win32` — both flavors are a faithful port of Node's `lib/path.js`,
differentially verified over 33,600 cases), `perf_hooks`, `process`, `punycode`,
`querystring`, `stream`,
`string_decoder`, `timers`(+`/promises`), `tty`, `url` (both the WHATWG `URL`
and the legacy `parse`/`format` API — the latter a faithful port of Node's
`Url.prototype.parse`/`.format`, differentially verified over 15,808 cases),
`util`(+`/types`), `v8`
(serialize = JSON, not V8 binary; heap stats are a shim), `async_hooks`
(AsyncLocalStorage sync-only; hooks are no-ops), `zlib`.

`process.emitWarning` writes to stderr in Node's format
(`(node:PID) [CODE] Name: message`, plus the one-time
`(Use \`node --trace-… ...\`)` hint), honoring `--no-warnings`,
`--no-deprecation`, `--trace-warnings` and `--trace-deprecation`. `url.parse`
emits `DEP0169` through it.

Known-but-UNIMPLEMENTED (require() returns a namespace so import-then-conditional
code loads; calling a method throws `Error: <mod>.<method> is not implemented in
node-js` — honest, never a silent fake): `tls`, `http2`, `https`, `worker_threads`,
`cluster`, `dgram`, `inspector`, `wasi`, `trace_events`, `domain`, `repl`, `vm`,
`readline`, `dns/promises` (use `require('dns').promises`). These need real
TLS/HTTP2/OS-threads/sandboxing substrate.

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
  `emit` now keep a real listener table instead of discarding registrations.

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

## `node -e` evaluates a Script, not a CommonJS module

Node wraps a `.js` FILE in the CommonJS wrapper, which makes a top-level `return`
legal; under `-e` it evaluates a Script, where the same `return` is a
`SyntaxError`. node-js accepts a top-level `return` in both, so `node -e
'return'` exits 0 here and 1 in Node. Only the `-e` path differs; running a file
matches.

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
as `0` and then reports the stray digit, exactly as V8 does). 87/87 differential
cases match.

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
   onto one shared byte store and moving all 54 `@@bytes`/`@@elems` call sites
   across 15 files onto it. That is a real refactor, not a patch — doing it
   partially would leave some views aliasing and others not, which is worse
   than the honest gap. `express.json()` does not depend on it (verified: a
   live POST round-trips byte-identically).

A builtin namespace (`require('buffer')`, `Buffer`, `fs`, …) enumerates the
members node-js ACTUALLY implements under `for…in` / `Object.keys` — not Node's
full export list. A package that clones a namespace key-by-key therefore gets
the working set rather than an empty object.


## Express (real npm package) — runs, serves HTTP, and parses request bodies
The real `express` 5.2.1 + its 65-package dependency tree loads and serves HTTP.
Verified end-to-end against `node v26.7.0` with the same app and the same `curl`
calls, byte-comparing every response body: `app.get`/routing/route params/query,
`res.send`/`res.json`/`res.status`, `app.listen`, **and** the body parsers —
`express.json()` (object, array, UTF-8, empty, and the malformed-input error
path returning `entity.parse.failed`), `express.urlencoded()`,
`express.text()` and `express.raw()`. All byte-identical.

The blockers were NOT the ones previously guessed here. They were:

1. **`async_hooks.AsyncResource` did not exist**, so `new AsyncResource(...)`
   threw `is not a constructor` inside `raw-body` and `on-finished`, which both
   wrap their callbacks with it.
2. **Builtin namespaces did not enumerate their members**, so `safer-buffer`'s
   `for (key in Buffer) Safer[key] = Buffer[key]` produced an empty object and
   `iconv-lite` then hit `isBuffer is not a function`.
3. **`String.prototype.indexOf` ignored its `fromIndex`**, so `body-parser`'s
   `parameterCount` never advanced past the first `&` and rejected every
   urlencoded body with `parameters.too.many`.

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

node-js translates the **overlapping** subset of JS regex that the Rust `regex`
crate can represent and **rejects the rest at RegExp-construction time** with a
`SyntaxError` — it never silently mis-executes a pattern.

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

**Rejected (construction throws `SyntaxError`, never a wrong match):** any pattern
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
- **`.index` on non-BMP input.** `exec`/`match` report the match position as a
  Unicode *char* offset; JS uses UTF-16 code-unit offsets, so an astral-plane
  character before the match shifts the index by one. Identical on BMP text.
## FIXED — collection access was O(n^2), and the cause was local to node-js

Per-element access to a JS array, object, or `Buffer` used to cost quadratic
time. An earlier revision of this file blamed `fusevm::Value::Array` holding a
by-value `Vec<Value>`. **That attribution was wrong.** node-js never constructs a
`fusevm::Value::Array` at all — its arrays are `JsObj::Array(Vec<Value>)` in its
own heap, reached through a `Value::Obj(u32)` handle (`src/host.rs:205`). The
fusevm `Arc`-backed array added in 0.19.0 therefore changed nothing here.

The real cause was four node-js sites that cloned an **entire heap cell just to
read its variant tag**, because `with_host` is a `RefCell` borrow and the code
inside each match arm re-enters the host, so a `&JsObj` borrow could not be held
across the match. `h.get(recv).cloned()` escaped the borrow — and deep-copied the
whole backing `Vec`/`IndexMap`/`String` every time:

| site | cost before |
| --- | --- |
| `get_property` (`src/builtins.rs:515`) | every property read copied the whole receiver, so `a[i]` and `a.length` were O(len) |
| `set_property` (`src/builtins.rs:1236`) | up to five whole-receiver copies per assignment |
| `call_method` (`src/host.rs:3023`) + `call_type_method` (`src/builtins.rs:4235`) | every `a.push(x)` copied the whole array before dispatching |
| `buffer::byte_get` / `byte_set` (`src/stdlib/buffer.rs:322`, `:345`) | one `buf[i]` materialised the whole buffer as a `Vec<u8>`; one `buf[i] = n` wrote every byte back |

The fix is not `Arc`/copy-on-write — JS arrays are mutable and aliased
(`const a=[1]; const b=a; b.push(2)` must be visible through `a`), and the heap
already gives correct aliasing because every handle points at one canonical cell.
The fix is to stop copying in order to *look*: `ObjKind` (`src/host.rs:274`) and
`JsHost::kind_of` (`src/host.rs:1158`) return the discriminant alone, and `peek`
(`src/builtins.rs:511`) hands back only the one field an arm needs. `push`/
`unshift` take their return length from the same mutable borrow via `array_len`
(`src/builtins.rs:4315`) instead of copying the array out to count it, and the
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

- **Symbol-keyed properties on an ARRAY are not enumerated.** An array's extra
  own properties live in the function-property side table rather than the
  object property map, so `const a=[1]; a[sym]=5` READS back correctly (`a[sym]`
  is `5`) but `console.log(a)` prints `[ 1 ]` where Node prints
  `[ 1, Symbol(k): 5 ]`, and `Object.getOwnPropertySymbols(a)` is empty where
  Node reports `Symbol(k)`. Object receivers do both correctly.

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
