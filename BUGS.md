# node-js — known gaps and unimplemented behavior

## Node core-module coverage
Implemented natively (verified vs node v26): `assert`(+`/strict`), `buffer`,
`child_process` (exec/spawnSync/execSync; `spawn` is sync-backed, not a live
streaming ChildProcess), `console`, `crypto` (hashes/hmac), `dns` (lookup/resolve
via std), `diagnostics_channel`, `events`, `fs`, `http`, `net`, `os`, `path`
(+`/posix`), `perf_hooks`, `process`, `punycode`, `querystring`, `stream`,
`string_decoder`, `timers`(+`/promises`), `tty`, `url`, `util`(+`/types`), `v8`
(serialize = JSON, not V8 binary; heap stats are a shim), `async_hooks`
(AsyncLocalStorage sync-only; hooks are no-ops), `zlib`.

Known-but-UNIMPLEMENTED (require() returns a namespace so import-then-conditional
code loads; calling a method throws `Error: <mod>.<method> is not implemented in
node-js` — honest, never a silent fake): `tls`, `http2`, `https`, `worker_threads`,
`cluster`, `dgram`, `inspector`, `wasi`, `trace_events`, `domain`, `repl`, `vm`,
`readline`, `dns/promises` (use `require('dns').promises`). These need real
TLS/HTTP2/OS-threads/sandboxing substrate. `fs/promises`, `stream/consumers`,
`path/win32` not yet aliased.


## Express (real npm package) — runs and serves HTTP; body-parsing gap
The real `express` 5.2.1 + its 65-package dependency tree loads and serves HTTP:
`app.get`/routing/route params/query, `res.send`/`res.json`/`res.status`, and
`app.listen` all match `node` byte-for-byte (verified via curl). Remaining gap:
`express.json()` / `express.urlencoded()` **request-body parsing** fails inside
`iconv-lite`, because node-js's `Buffer` is a plain `@@native` object rather than
a real **`Uint8Array` subclass** — so `buf instanceof Uint8Array` is `false` and
`iconv`/`safer-buffer`'s Buffer-static copy loop misses `isBuffer`. Making
`Buffer` a genuine `Uint8Array` subclass (and enumerating builtin-namespace
static keys) is the next chunk of work. GET/response-side Express is unaffected.


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
  coroutines on the shared thread-local heap).
- **Iterators** — honoring `Symbol.iterator` in `for-of`/spread for user
  iterables; array/string/Map/Set/generator iterators with `.next()`.
- **Labeled statements** — `outer: for (...) { ... continue outer / break outer }`
  bind `continue`/`break` to the labeled loop target (compiler.rs). Verified
  against `node v26.5.0`: labeled `continue`/`break` retarget the correct loop.
- **Promises + async/await + event loop** — `new Promise`, `.then`/`.catch`/
  `.finally`, `Promise.resolve`/`reject`/`all`/`allSettled`/`race`/`any`; `async`
  functions/arrows/methods, `await`, rejection-as-throw; a host-driven loop
  draining `process.nextTick` → promise microtasks → timers
  (`setTimeout`/`setInterval`/`setImmediate`, `queueMicrotask`), Node ordering.

## Not implemented (parse/compile-time error, no silent wrong answer)

These are absent from the lexer/parser/compiler; a program using them fails
loudly rather than producing a wrong result. The fuzzer does not generate them.

- **`-x ** y`** — JS makes an unparenthesized unary minus directly before `**` a
  `SyntaxError` (`(-x) ** y` or `-(x ** y)` is required). node-js's parser accepts
  it and evaluates `-(x ** y)`. Applies to both Number and BigInt; the fuzzer
  parenthesizes the base to avoid generating this ambiguous form.

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

- **Nested-object/array width-based line breaking.** `util.inspect` array
  *grouping* (>6 elements) is ported faithfully, but the general `breakLength` 80
  wrapping of nested objects — where a long single-line object/array is broken
  onto multiple lines — is not modelled. A `>6`-element array nested inside an
  object may also render at the wrong indentation. Top-level arrays match Node.

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
- **`Number.prototype.toFixed`/`toPrecision`** — round half away from zero on the
  exact value, preserve the sign of a zero result, keep full precision at large
  magnitudes.
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
