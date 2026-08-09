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

A `Buffer` is a real `Uint8Array` subclass instance:
`Object.getPrototypeOf(buf) === Buffer.prototype` holds, that prototype is a
genuine object whose own `[[Prototype]]` is `Uint8Array.prototype`, and a
Buffer's own keys are its byte indices (`Object.keys`, `for…in`,
`getOwnPropertyNames`, `hasOwnProperty`, `getOwnPropertyDescriptor` and object
spread all agree). One divergence remains: Node's `Buffer.prototype` methods are
*enumerable*, so `for (k in buf)` in Node yields the ~100 prototype method names
after the byte indices. node-js implements a subset of those methods and marks
them non-enumerable, so `for (k in buf)` yields the indices only. Emitting our
shorter list instead would advertise methods Node has that node-js does not.

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

`Object.prototype.toString.call(buf)` is now `[object Uint8Array]` (and every
other builtin exotic brands correctly), `ArrayBuffer.isView(buf)` is `true`, and
`buf.byteLength`/`byteOffset`/`BYTES_PER_ELEMENT`/`set` exist. A `Buffer` is
still a `@@native`-tagged object rather than a genuine `Uint8Array` subclass
INSTANCE, so `Object.getPrototypeOf(buf) === Buffer.prototype` is `false`; no
observed package depends on that identity.


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
## Partial / simplified semantics (runs, but not byte-identical to node in edge
cases the fuzzer is scoped away from)

- **`util.inspect` `compact` depth-gate (only under a raised `{depth}`).** Node
  forces an object/array onto multiple lines when its deepest descendant is `≥
  compact` (3) levels below it, even if the single-line form fits `breakLength`.
  Only the `breakLength` 80 fit is modelled, not the depth-gate, so
  `util.inspect(x, {depth: N>2})` on a `≥4`-level-deep structure may stay on one
  line where Node breaks it. `console.log` (fixed `depth: 2`, where the gate can
  never fire) is byte-identical.

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
  `true`). (`Object.prototype.toString.call(x)` still returns `[object Object]`
  for these — a separate, pre-existing `Symbol.toStringTag` gap shared by
  `Map`/`Set`/`Promise`/`Date`/`URL`.)
- **`FinalizationRegistry`** — constructor requires a callable; `register(target,
  held[, token])` and `unregister(token)` enforce their `TypeError`s and
  `unregister` returns the correct boolean. Cleanup callbacks never fire because
  the heap holds every value strongly (a spec-permitted approximation, same basis
  as `WeakRef`).
