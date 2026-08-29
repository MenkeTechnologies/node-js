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

One ECMAScript global is absent entirely, and references as `ReferenceError`
rather than pretending:

- **`Intl`.** Needs ICU (locale-aware number/date/collation data). There is no
  honest subset: a `Intl.NumberFormat` that only handles `en-US` would give
  wrong answers for every other locale instead of an error.

`Proxy` is implemented — see "Proxy" below for the shape and the two divergences
that remain.

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

## Array holes: what is and is not modelled

An ELIDED array element (`[1,,3]`, `new Array(5)`, `delete arr[i]`, an index
written past the end, a `length` grow) is tracked as a real hole, not as a
stored `undefined`. The marker is deliberately NOT a `Value` variant: a sentinel
would have to be mapped back to `undefined` at every element read, and one
missed read would leak an un-nameable value into user code. Instead the elision
set is a side table on the host keyed by heap index (`JsHost::array_holes`), and
the element vector still holds an ordinary `Value::Undef` at a hole — so there
is no sentinel that CAN leak, and a code path that has not been taught about
holes degrades to the plain `undefined` reading rather than to something
unrepresentable.

Modelled and verified against node v26.7.0: `in`, `Object.keys`/`values`/
`entries`/`assign`/`getOwnPropertyNames`/`getOwnPropertyDescriptor`,
`hasOwnProperty`, `propertyIsEnumerable`, `for…in`, object spread, `length`
after a `delete`, `structuredClone`, the `<N empty items>` `util.inspect`
rendering at every nesting depth, the `HasProperty`-spec'd iteration methods
that SKIP a hole (`forEach`/`map`/`filter`/`some`/`every`/`reduce`/
`reduceRight`/`flat`/`flatMap`/`indexOf`/`lastIndexOf`/`sort`) versus the
`Get`-spec'd ones that see the `undefined` it reads back as (`for…of`, spread,
`join`, `includes`, `find`, `Array.from`, `entries`), and hole tracking through
`push`/`pop`/`shift`/`unshift`/`reverse`/`fill`/`copyWithin`/`splice`/`slice`/
`concat`/`map` and the change-by-copy methods.

An array-LIKE receiver taken through `Array.prototype.<m>.call` is holed by the
same rule: an index the object does not own is an elided element of the
temporary the generic path builds (see `array_generic` in `src/builtins.rs`).

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

## The temporal dead zone is a missing binding, not a poisoned one

A `let`/`const`/`class` binding is created where its declaration runs rather
than where its block is entered, so the dead zone before it is indistinguishable
from a name that was never declared. A plain read throws `ReferenceError` either
way and so looks right, but the two observable halves are wrong:

```js
try { d; let d = 1 } catch (e) { e.message }
// node:    Cannot access 'd' before initialization
// node-js: d is not defined

try { typeof d; let d = 1 } catch (e) { "threw" }
// node:    threw          (typeof does NOT rescue a name in the dead zone)
// node-js: no throw       (typeof reads it as an unbound name -> "undefined")
```

Both follow from the same absence. `typeof` is the sharper one: it is the only
read JS allows of an unresolvable name, and the dead zone is the one case where
it still throws, so treating "not there yet" and "never declared" alike gets it
backwards. Closing this needs what `var` hoisting now does, but per BLOCK and
with a poison rather than `undefined`: the lexical names of a block created on
entry holding a marker, every name read checking for it, and the declaration
overwriting it. The check lands on the by-name read path — which is the path
`slots.rs` exists to keep hot code off, so the cost is bounded — but it is a
representation change to every binding, and it is not free enough to fold into
an unrelated fix.

## FIXED — `var` bindings exist from scope entry, not from their declaration

A `var` used to be created where its declaration ran, so every read above it
threw instead of answering `undefined`, and a bare `var x;` overwrote a binding
that already existed:

```js
function f() { console.log(x); var x = 1 }   // was: ReferenceError: x is not defined
function g(a) { var a; return a }; g(5)      // was: undefined
```

Scope entry now creates them. `compiler::hoist_vars` walks the statements of one
function scope — descending through blocks, loop heads, `switch`, `try`/`catch`/
`finally` and labels, because none of those scopes a `var`, and stopping at a
nested function, which starts a scope of its own — and emits `ops::HOIST_VAR`
per name, destructuring targets included. The op creates the binding as
`undefined` only when it is absent, which is what leaves a parameter standing,
and a bare `var x;` now emits nothing at its own position. It runs before the
function-declaration hoisting so a `function h(){}` overwrites a `var h`, the
order the spec instantiates them in. A slotted local is skipped: its slot already
reads `undefined` before its first write.

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

## FIXED — `sort` was O(n^2), and `undefined` reached the comparator

`Array.prototype.sort` (and `toSorted`, and the typed-array `sort` with a user
comparator) was an insertion sort: `sort_values` in `src/builtins.rs` walked
each element back through its predecessors one swap at a time. Every doubling of
the input quadrupled the work. Sorting 200,000 numbers with a comparator did not
finish inside 120 s; node v26.7.0 sorts the same array in 70 ms.

Release build, same machine (Apple M5 Max), `a.sort((p, q) => p - q)` over a
pseudo-random array:

| n | before | after |
| --- | --- | --- |
| 1,000 | 0.21 s | 0.012 s |
| 2,000 | 0.81 s | 0.022 s |
| 4,000 | 3.39 s | 0.043 s |
| 8,000 | 12.94 s | 0.086 s |
| 16,000 | 51.36 s | 0.179 s |
| 200,000 | did not finish in 120 s | 2.97 s |

4x per doubling before, 2x after. The default (no-comparator) order was
quadratic for the same reason — 0.039 / 0.113 / 0.444 s at n = 2,000 / 4,000 /
8,000 — since both orders went through the same loop.

`sort_values` is now a bottom-up stable merge sort. Bottom-up rather than
recursive because the JS comparator runs on the same native stack. Ties take
from the left run, which preserves the stability the spec requires:
`[{k:1},{k:0},{k:1},{k:0}].sort((x,y)=>x.k-y.k)` keeps the two `k:0` entries in
input order. A comparator result of NaN is not `> 0`, so such a pair keeps its
order too, as `[1,2,3].sort(() => NaN)` does in node.

The rewrite also fixed a semantic divergence the insertion sort had: 23.1.3.30.1
SortIndexedProperties never passes `undefined` to the comparator — it is moved
after every defined value once they are ordered.

| expression | node v26.7.0 | node-js before | node-js now |
| --- | --- | --- | --- |
| `[3,undefined,1].sort((x,y)=>x-y)` | `[1,3,undefined]` | `[3,undefined,1]` | `[1,3,undefined]` |
| comparator calls for that sort | 1 | 2 | 1 |

The typed-array path had its own copy of the insertion sort in
`src/stdlib/typedarray.rs`, with a `<= 0.0` break that turned a NaN comparator
result into a SWAP rather than "keep this order" (23.2.4.1 step 3 reads NaN as
+0). It now converts to `Value::Float`s and calls the same `sort_values`, so
there is one sort in the runtime and one set of semantics.

`tests/es_parity.rs` guards this by COUNTING comparator calls, not by timing:
sorting 4,096 reversed elements must stay under `n * 12` calls. The merge sort
uses 24,576; the insertion sort used 8.4 million. node uses 4,095 — TimSort
detects the reversed run — which is why the bound is a bound and not an equality.

## FIXED — a `String` allocated for every variable read, and a `TRUTHY` call per loop condition

Every identifier in this runtime is a name lookup: the compiler lowers `x` to
`CallBuiltin(GETLOCAL)` with the name as a string constant, and the host walks
the scope chain for it. Two costs on that path were pure waste.

`sval` (`src/builtins.rs`) deep-copied the name out of its `Arc<String>` on
every read, write, and declaration — a heap allocation and a memcpy per variable
access, so several per loop iteration. `sname` clones the `Arc` handle instead.
`JsHost::set_name` (`src/host.rs`) hashed the name twice and allocated a fresh
`String` key to overwrite a binding that was already there; it now writes
through `get_mut`.

`compile_condition` (`src/compiler.rs`) wrapped every `if`/`while`/`for` test in
`CallBuiltin(TRUTHY)` even when the test had just produced a `Value::Bool`.
`yields_bool` skips the call for the relational and equality operators, `in`,
`instanceof`, `delete`, `!`, and the boolean literals — one fewer host
round-trip per condition evaluation, which in a counting loop is one per
iteration. It also places the comparison immediately before the jump that
consumes it, which is what fusevm's block JIT requires of a bool-producing op.

Release build, wall clock, mean of 8 hyperfine runs:

| workload | before | after |
| --- | --- | --- |
| 5M-iteration counting loop | 2400 ms | 1895 ms |
| `fib(27)` | 722 ms | 651 ms |
| 1M `Math.sqrt` / divide | 822 ms | 693 ms |
| 300k array map/filter/sum | 665 ms | 624 ms |
| 200k `Map` set + get | 457 ms | 431 ms |

Not tried again: hashing the scope maps with `FxHash` instead of the default.
It was measured and it is SLOWER — `fib(27)` went 651 ms to 1086 ms and the
counting loop 1895 ms to 2381 ms — so `VarMap` (`src/host.rs`) keeps the default
hasher.

What none of this touched at the time: every one of these programs ran entirely
in the fusevm interpreter. That is settled for the counting loop by the loop
ROTATION below (`node --tiers` now reports `reaches native code true` for it);
the workloads whose bodies still hold a `CallBuiltin` — `Math.sqrt`, `Map.get`,
a callback per element — remain interpreted, because fusevm's tiers decline any
region containing one.

## FIXED — a whole VM, and a copy of the function's bytecode, per call

Every JS call, every `try` block and every generator step runs its chunk through
`host::run_chunk_on`, which built a `fusevm::VM` from scratch each time: three
`Vec` allocations, 70 `register_builtin` writes, an `Arc` for the numeric hook
and the JIT enable. `fib(27)` makes ~400,000 calls, so it built 400,000 VMs to
run a 23-op body.

The caller also had to hand over an OWNED `Chunk`, and `run_user_func_nt` got
there by cloning the whole `FuncDef` — so each call also deep-copied the
function's entire compiled body: `ops`, `constants`, `names`, `lines`,
`sub_entries`, `block_ranges`, `source`, and `sub_chunks` recursively. Running a
`try` cloned its `TryDef` the same way: block, handler and finalizer bytecode,
once per entry, which inside a loop is once per iteration.

Both are now pooled. `VM_POOL` (`src/host.rs`) keys idle VMs by the chunk they
still hold — `func_key(def_id)` for a function body, `try_key(try_id, part)` for
one part of a `try`. A repeated call takes back the VM that already holds that
chunk and hands it straight back to `VM::reset`, which keeps the builtin table,
the hooks and the JIT setting; no bytecode is copied at all. A key's stack grows
to the deepest simultaneous entry into that function and no further, so a
recursion 27 deep holds 27 VMs rather than building 400,000.

`run_user_func_nt` now reads only the light fields of a `FuncDef` (params,
generator/async/arrow flags, name); the chunk is reached exactly once per pooled
VM. `b_try` (`src/builtins.rs`) reads `JsHost::try_shape` — has-handler, catch
parameter name, has-finalizer — instead of cloning three chunks to learn it.

Minimum-of-4 user+sys CPU seconds, release build, same machine. CPU time rather
than wall clock because this machine runs many builds at once and a loaded
wall clock says nothing:

| workload | before | after |
| --- | --- | --- |
| `fib(27)` | 0.94 s | 0.65 s |
| 200k-element `sort` with a comparator | 3.48 s | 2.41 s |
| 300k array `map`/`filter`/sum | 0.81 s | 0.59 s |
| 300k method calls building objects | 1.68 s | 1.34 s |
| 5M-iteration counting loop | 2.51 s | 2.52 s |
| 200k `Map` set + get | 0.58 s | 0.58 s |

Call-heavy work gains 1.25–1.45x; a loop that never calls anything is unchanged,
as expected — it runs one chunk.

## Kept but NOT a speedup — eliding the per-iteration loop scope

`for (let i = …)` re-binds its head per iteration (ForBodyEvaluation's
CreatePerIterationEnvironment) so a closure made in one pass keeps that pass's
value, and `{ … }` opens a scope for its lexical declarations. A profile of a
5M-iteration counting loop put 17% of its samples in `copy_scope` and the
`EnvData` allocate/free traffic beneath it — copies that, with no closure
anywhere in the loop, nothing could observe.

`src/capture.rs` answers "can this subtree hold on to the scope it runs in?"
(a function, a class, or a direct `eval`), and the compiler skips the
per-iteration copy and the block scope when the answer is no. It is CORRECT —
`tests/es_parity.rs` pins the capturing cases against node — but it is not
faster: measured old-vs-new binaries in the same run, 1.01x on the counting
loop, 1.07x on array work, and inside the noise everywhere else. The allocation
it removes is not what the loop spends its time on. It stays because it is the
same question slot-resolved locals have to ask (pythonrs asks it as
`fn_slots_allowed`), not because it paid for itself here.

What the loop spends its time on is the host round-trip per operation. An empty
`for (let i = 0; i < 5_000_000; i++) {}` costs 178 ns per iteration for four
`CallBuiltin`s — `GETLOCAL i`, `NUM_STEP`, `SETLOCAL i`, plus the comparison —
against node's 6 ns for the same loop. Every identifier in a node-js program is
a name lookup through the host: pop the name off the VM stack, borrow the
thread-local `JsHost`, walk the `Rc<RefCell<EnvData>>` chain, hash the string,
clone the value back. Nothing under that is a tuning problem; the fix is to stop
emitting name lookups for locals and give them fusevm frame slots
(`Op::GetSlot`/`SetSlot`, which is also what makes a block JIT-eligible).

## FIXED — locals were name lookups; the ones nothing can reach are frame slots now

Every identifier in a node-js program used to be a hash lookup through the host.
`let s = 0; for (let i = 0; …)` compiled to `CallBuiltin(GETLOCAL)` /
`CallBuiltin(SETLOCAL)` with the name as a string constant, and each one popped
the name off the VM stack, borrowed the thread-local `JsHost`, walked the
`Rc<RefCell<EnvData>>` chain and hashed the name in every scope on the way. An
empty `for (let i = 0; i < 5_000_000; i++) {}` cost 178 ns per iteration for four
of those round-trips; node runs the same loop at 6 ns.

`src/slots.rs` decides which locals can live in the frame slot vector fusevm
already keeps, addressed by index. The rules are conservative, because a name
slotted in one place and looked up by name in another is a silent wrong answer:

1. A name any OTHER chunk can reach keeps its binding. Nested functions, arrows,
   class bodies and `try` parts each compile to their own chunk and resolve what
   they name through the environment chain, so every identifier they mention is
   off the table — but only those identifiers. A counting loop still gets slots
   in a file that also passes a callback to `map`. A direct `eval` can name
   anything at run time, so it disables the chunk outright.
2. One declaration per name; a shadowed name is left alone rather than
   scope-tracked.
3. No read before the declaration, in source order. A slot reads as `undefined`
   before its first write, which is neither node's `ReferenceError` for a `let`
   in its temporal dead zone nor the `ReferenceError` node-js answers today, so
   a name whose first mention is a read stays where those answers come from.
4. Simple identifiers only — a destructuring target or `delete x` binds through
   the host.
5. At the top level of a script, `let`/`const` only: a top-level `var` is a
   property of the global object (`var g = 1; globalThis.g` is `1`).

A parameter arrives in the call environment, so the function prologue copies each
slotted one into its slot once and every later use is a bare `GetSlot`.
Generators, async functions and `--dap` runs keep names — the first two suspend
the frame the slots live in, and the debugger reads scopes by name.

The counting loop's body is now `GetSlot / GetSlot / Add / Dup / SetSlot / Pop`,
with no host round-trip at all; what is left in it is the `NUM_STEP` builtin
behind `i++` (which is `ToNumeric`, so it is BigInt-aware) and the loop's own
`PUSH_SCOPE`/`POP_SCOPE` pair, once per loop rather than per iteration.

Minimum-of-4 user+sys CPU seconds, release build (CPU time because this machine
runs many builds at once):

| workload | before | after |
| --- | --- | --- |
| 5M-iteration `s += i` | 1.90 s | 0.88 s |
| 5M-iteration counting loop with `% 7` | 2.05 s | 1.06 s |
| 500k property reads | 0.33 s | 0.17 s |
| 1M `Math.sqrt` / divide | 0.62 s | 0.42 s |
| 200k `Map` set + get | 0.56 s | 0.49 s |
| 500k plain function calls | 0.73 s | 0.63 s |

Against node on the same machine, the 5M `s += i` loop went from 33.8x slower to
14.7x, and 500k property reads from 10.3x to 5.7x. Callback-driven work
(`sort` with a comparator, `map`/`filter` chains, method dispatch) is unchanged:
its time is in `host::invoke` per element, not in name lookups.

Verified with the 120-script parity corpus (byte-identical to node v26.7.0),
4,000 differential fuzzer cases (0 divergences) and a new `es_parity` test
covering parameters, defaults, rest, `arguments`, shadowing, a closure over a
loop variable, `try`, generators, `typeof` and calling through a slotted
binding.

Not yet: `reaches native code` is still `false`. The loop region holds one
`CallBuiltin` (`NUM_STEP`) and fusevm's block tier declines any region
containing one, so the JIT still never compiles it.

## FIXED — `i++` on a proven-Number local, and what still keeps the JIT away

The counting loop's last host round-trip was `i++`. It lowered to
`CallBuiltin(NUM_STEP)`, which exists because `x++` is `ToNumeric(x)` and has to
keep a BigInt a BigInt (`1n++` is `2n`, not `2`).

A slot the compiler can prove holds a Number needs none of that: `ToNumeric` is
the identity on it, and `Number ± 1` is a Number. `crate::slots` tracks that
proof — a local declared from a numeric literal and afterwards written only by
`++`/`--` — and the update lowers to `GetSlot`, a native `Add`, `SetSlot`. Any
other write (`i = something`, `i += n`, a `for…of` binding, a parameter, a
BigInt or string initializer) drops the name out of the numeric set and keeps
the builtin, so `let b = 1n; b++` is still `2n`.

A 5M-iteration `s += i` loop is now, in full:

    GetSlot(1) LoadFloat NumLt JumpIfFalse
    GetSlot(0) GetSlot(1) Add Dup SetSlot(0) Pop
    GetSlot(1) Dup LoadFloat(1) Add SetSlot(1) Pop
    Jump

with no `CallBuiltin` in the loop at all, where it used to hold six.

**The JIT did not compile it, and the reason was the loop's SHAPE.** `node
--tiers` reported the loop `trace-eligible=true` (it had been `false` — every
`CallBuiltin` in the body disqualified it), which is the precondition for
fusevm's trace tier. But the tracer never installed a trace: `traced=false`,
`reaches native code false`. That is fixed below.

## Which loops reach native code, and what stops the rest

With the rotation and the `for (;;)` back edge below, loop SHAPE no longer
keeps anything out of fusevm's tracing JIT. What is left is the op content of
the body, and `--tiers` names the two cases distinctly — the distinction is
worth reading before optimizing anything:

- `traced=false` with `trace-eligible=true` was the SHAPE problem. No loop form
  reports that any more.
- `trace-eligible=false` is the OP problem: the body holds a `CallBuiltin`, and
  fusevm's tiers decline any region containing one. Rotation is irrelevant to
  it.

Measured on this frontend (debug build, `--tiers`):

| loop | reaches native code |
| --- | --- |
| `for (let i = 0; i < n; i++) s += i` | yes |
| `while (i < n) { s += i; i++ }` | yes |
| `do { s += i; i++ } while (i < n)` | yes |
| `for (;;) { s += i; i++; if (…) break }` | yes |
| `for (const v of a) s += v` | no — `trace-eligible=false` |
| a body calling `Math.sqrt`, `Map.get`, `arr.push`, or reading `o.x` | no — `trace-eligible=false` |

`for…of` is a structural case rather than a missing optimization: its step is
`FORITER`, a host call that runs the iterator protocol, so the loop can never
present a builtin-free body while the protocol is hosted. The others are the
ordinary JS-specific lowerings (`CallBuiltin(BINOP)` for `&`, `GETATTR` for a
property read); each would need its own proof that the JS coercion is
unnecessary before it could become a native op.

Two gates checked here and found NOT to apply, so they are not worth
re-investigating on this frontend: a statement-position `if` inside a loop does
not make it trace-ineligible (`--tiers` reports `traced=true` for a counted loop
whose body is a bare `if`), and no loop form emits a stack-imbalancing
`LoadUndef`.

## FIXED — every `require` re-walked the filesystem, cached module or not

`module::require` resolved its specifier on every call: a `node_modules` walk
for a bare name, a set of extension probes for a relative one, then
`std::fs::canonicalize`. Only AFTER all of that did it consult the module cache
— so a program that requires the same specifier in a loop, or a dependency graph
where twenty modules require the same helper, paid the full filesystem cost
every time for a module that was already loaded.

Resolution is now memoized per `(specifier, from_dir)`. Node keeps the same
table (`Module._pathCache`) with the same consequence: a file that appears after
a specifier has already resolved is not picked up by a later `require` of it.

Measured on debug builds, wall clock, the baseline built from `git archive` of
the parent commit into its own tree:

| workload | before | after | |
| --- | --- | --- | --- |
| 12,000 cached `require`s (40 modules x 300) | 599-633 ms | 380-406 ms | 1.57x |
| cold `require('express')` (200 files) | 475-590 ms | 460-534 ms | ~1.05x |

The cold load is dominated by parsing and compiling ~3.4 MB of JavaScript, which
this does not touch. Bytecode-caching those modules was tried in an earlier
round and MEASURED SLOWER — deserializing 118 bincode blobs cost more than
recompiling — and reverted; do not retry it without a new measurement.

## FIXED — every `for` and `while` was lowered in a shape the tracing JIT refuses

fusevm's tracing JIT only closes a trace on a CONDITIONAL backward branch.
`compile_while` and `compile_for` emitted the test at the TOP and closed the
loop with an unconditional `Jump` back to it, so the recorder recorded those
loops and then declined them — `--tiers` said `trace-eligible=true traced=false`
and `reaches native code false` for every one, and a hot loop stayed in the
interpreter however hot it got. The one loop form that already ended in a
conditional branch, `do { … } while ( … )`, traced fine, which is what made the
cause visible: the same arithmetic ran 288x faster written that way.

Both are now emitted ROTATED: the test once as an entry guard, once at the
bottom as a conditional backward branch. `continue` targets the bottom copy
(after the update clause, for a C-style `for`).

`for (;;)` has no test to branch on, and emitting the honest unconditional
`Jump` left it interpreted: fusevm's trace compiler only ever installs a trace
closed by `JumpIfTrue`/`JumpIfFalse` and silently declines a `Jump` close, so
`for (;;)` stayed in the interpreter while the identical `while (true)` — whose
condition already lowered to `LoadTrue; JumpIfTrue` — reached native code. It
now closes with that same constant-true conditional branch. Measured on a debug
build, user CPU, 3M iterations of `s += i`: **4.26 s to 0.01 s**, with `--tiers`
going from `traced=false` to `traced=true`.

Evaluation order and count are unchanged: a top-test loop runs the test `n + 1`
times for `n` iterations, and so does this. Rotation costs one copy of the
condition's code and saves one jump per iteration.

Measured on debug builds, user CPU, best of 7 with the two binaries interleaved
so machine load cannot favour either. Both sides were built from
`git archive` of the exact commits — the rotation commit and its parent —
extracted to separate trees with their own target directories, rather than by
reverting in place: a worktree that has been through a `git stash pop` on this
box can report clean while the files differ from HEAD, so `git status` is not a
usable baseline gate.

| workload | unrotated | rotated | |
| --- | --- | --- | --- |
| 5M `s += i` in a function | 7.89 s | 0.02 s | 394x |
| `fib(27)` | 4.17 s | 4.08 s | 1.02x |
| 1M `Math.sqrt` / divide | 4.99 s | 4.96 s | 1.01x |
| 300k array map/filter/sum | 3.31 s | 2.86 s | 1.16x |
| 200k `Map` set + get | 1.46 s | 1.50 s | 0.97x |
| 500k property reads | 1.52 s | 1.44 s | 1.06x |
| 5M `s += i & 7` | 9.79 s | 9.45 s | 1.04x |
| 200k string appends | 2.87 s | 2.84 s | 1.01x |

Only the first reaches native code; the rest sit within a few percent either
way, which is the expected result — rotation changes which loops the tracer can
compile, not how the interpreter runs the ones it cannot.

What still keeps the others interpreted is the `CallBuiltin` in their bodies.
The nearest of them is the bitwise loop: JS `a & b` is
`ToInt32(a) & ToInt32(b)`, which lowers to `CallBuiltin(BINOP)` even though
fusevm has a native `Op::BitAnd`. Using the native op would need the ToInt32
coercion proven away first, and that has not been done.

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

## FIXED in round 6 — `assert`'s error class and generated messages

`e.name` was `Error` and `e.message` was
`AssertionError [ERR_ASSERTION]: 1 strictEqual 2`. Two separate faults:

1. `"AssertionError"` was not in `host::ERROR_NAMES`, so `synth_error` failed
   the class check on the `AssertionError [ERR_ASSERTION]: …` head and fell into
   the `Error` branch keeping the WHOLE head as the message — a prefix node
   keeps out of `.message` entirely (it appears only in `.stack`). It is a
   recognized name now, and still NOT a global: node exposes the class only as
   `assert.AssertionError`, and `GLOBAL_FUNCS` is a separate table.
2. Every comparison generated `{actual} {op} {expected}`, so `strictEqual(1, 2)`
   produced `1 strictEqual 2` — the operator NAME substituted where node writes
   a heading. That shape is correct only for the two loose forms.

Each comparison now carries node's own shape, measured on v26.7.0:

| call | node v26.7.0 `.message` |
| --- | --- |
| `equal(1,2)` | `1 == 2` |
| `strictEqual(1,2)` | `Expected values to be strictly equal:\n\n1 !== 2\n` |
| `notStrictEqual(1,1)` | `Expected "actual" to be strictly unequal to: 1` |
| `deepEqual(x,y)` | `Expected values to be loosely deep-equal:` + both sides |
| `notDeepStrictEqual(x,x)` | `Expected "actual" not to be strictly deep-equal to:` + the value |

Two residues, both narrower than the wording gap they replaced:

- **`ok(false)`** — node appends an echo of the failing SOURCE LINE
  (`\n\n  a.ok(false)\n`), which needs the call site's text. The heading matches.
- **A non-primitive operand renders single-line.** Node's assert calls
  `util.inspect` with `compact: false`, so `{a: 1}` becomes `{\n  a: 1\n}`;
  here it stays `{ a: 1 }`. The message STRUCTURE matches; only the inspect
  mode differs. `deepStrictEqual` between two objects additionally renders a
  `+ actual - expected` structural diff, which needs a line-oriented differ over
  inspect output and is not reproduced; its primitive form is exact.

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

## FIXED in round 7 — the divergences user code cannot catch

Every entry below was a `fatal runtime error: stack overflow` abort (exit 134),
a hang, or a silent success where node throws. A panic is a parity divergence
even when the happy path matches, because `try`/`catch` cannot see it.

### The native stack is bounded, and running out of it is a `RangeError`

A JS call is a Rust recursion — `host::run_user_func_nt` pushes a `Frame`, then
`run_chunk_on` builds a whole `fusevm::VM` on the stack and runs the body, whose
own calls land back there. Nothing bounded that, so unbounded JS recursion
walked off the end of the OS stack and killed the process. Two changes:

- **`main` runs the program on a 256 MiB thread** (`lib.rs::run_on_js_stack`),
  and each generator/async coroutine gets 16 MiB instead of corosensei's 1 MiB
  default. Both are `PROT_NONE` reservations faulted in on use, and a refused
  reservation falls back rather than failing. On the OS default 8 MiB stack a
  debug build managed **83** frames — measured, `node -e 'function
  f(n){if(n<=0)return 0;return 1+f(n-1)} f(84)'` aborted.
- **`host::stack_exhausted` throws before the stack runs out**, comparing the
  live stack pointer against a floor derived from the RUNNING stack's real
  bounds (pthread on the entry thread, `Stack::limit()` on a coroutine). A frame
  count would have been wrong: a node-js frame is ~98 KiB in a debug build and
  far less in release.

| case | node v26.7.0 | node-js before | node-js now |
| --- | --- | --- | --- |
| `function f(){return f()}; f()` | `RangeError: Maximum call stack size exceeded` | abort, exit 134 | same `RangeError`, catchable, `instanceof RangeError` |
| `({valueOf(){return this+1}}) + 1` | same `RangeError` | abort | same `RangeError` |
| `p={toString(){return String(p)}}; String(p)` | same `RangeError` | abort | same `RangeError` |
| deep recursion inside a `function*` body | same `RangeError` | abort at ~10 frames (1 MiB coroutine stack) | same `RangeError` |
| `a=[1]; a.push(a); a.flat(Infinity)` | same `RangeError` | abort | same `RangeError` |
| `f(1000)` (ordinary recursion) | `1000` | abort above 83 | `1000` |

A finished generator now releases its coroutine — and its stack mapping — as
soon as the body returns. `host.generators` only ever grows, so without that a
20 000-iteration `await` loop held 20 000 stack reservations for the life of the
process.

### `Array.prototype.join` cuts its own cycle

`join`, `toString` and `toLocaleString` are the one graph walk the language
leaves unbounded, and every engine keeps a JoinStack: a receiver already being
joined contributes the EMPTY STRING instead of recursing. node-js had no such
cut and aborted. Only RE-ENTRANCE is cut, not repetition.

| case | node v26.7.0 | node-js before |
| --- | --- | --- |
| `a=[1]; a.push(a); a.push(2); a.join('-')` | `"1--2"` | abort |
| `d=[]; d.push(d); String(d)` / `d.toString()` / `` `${d}` `` | `""` | abort |
| `e=[1]; e.push(e); [e,e].join('|')` | `"1,|1,"` | abort |
| `g=[1]; g.push(g); g.toLocaleString()` | `"1,"` | abort |

### A Map/Set inspects at its nesting depth

Both rendered their members through `inspect`, which restarts at indent 0, so
the depth gate never fired. Two consequences, one cosmetic and one fatal: nested
Maps printed a level too deep at every depth, and a self-referential Map or Set
recursed until the process aborted.

| case | node v26.7.0 | node-js before |
| --- | --- | --- |
| four nested `Map`s | `Map(1) { 'a' => Map(1) { 'b' => Map(1) { 'c' => [Map] } } }` | one level deeper, no `[Map]` |
| `new Map([['k',[1,[2,[3,[4]]]]]])` | `Map(1) { 'k' => [ 1, [ 2, [Array] ] ] }` | `[ 1, [ 2, [ 3, [Array] ] ] ]` |
| `m=new Map(); m.set('m',m); console.log(m)` | `<ref *1> Map(1) { 'm' => [Circular *1] }` | abort |

### Length arithmetic is checked before the allocation

| case | node v26.7.0 | node-js before |
| --- | --- | --- |
| `'a'.repeat(2**40)` | `RangeError: Invalid string length` | hang — building a 1 TiB `String`, killed at 10s |
| `'abc'.padStart(2**40,'x')`, `'ab'.padEnd(536870889,'x')` | same | hang |
| `new Array(2**32)`, `a.length = 2**32` | `RangeError: Invalid array length` | hang — four billion elements |
| `new Array(-1)`, `new Array(1.5)` | same | `[ -1 ]` / `[ 1.5 ]` — the argument became an ELEMENT |
| `a.length = -1`, `a.length = 'x'` | same | silently ignored |

`host::MAX_STRING_LENGTH` is `536870888` — V8's `String::kMaxLength` on a 64-bit
build, and the value `require('buffer').constants.MAX_STRING_LENGTH` reports on
node v26.7.0. What is bounded is the RESULT, so `''.repeat(2**53)` is still `''`
and `'ab'.padStart(2**40,'')` is still `'ab'` (the empty-filler short-circuit
comes first, as it does in V8). The array test is `ToUint32(len) === ToNumber(len)`
(10.4.2.4), so `a.length = '3'` is `3` and `new Array(-0).length` is `0`.

### Error SHAPE: the constructor, not the message

Each of these was a silent SUCCESS in node-js — a right-looking program with no
error at all, which no message audit can see.

| case | node v26.7.0 | node-js before |
| --- | --- | --- |
| `Object.create(1)` / `('s')` / `(undefined)` | `TypeError: Object prototype may only be an Object or null: …` | built a normal object |
| `Object.defineProperty(1, 'a', {})` | `TypeError: Object.defineProperty called on non-object` | returned, wrote nothing |
| `Object.defineProperty({}, 'a', 1)` | `TypeError: Property description must be an object: 1` | returned, wrote nothing |
| `Object.setPrototypeOf(null, {})` | `TypeError: Object.setPrototypeOf called on null or undefined` | returned `null` |
| `Object.setPrototypeOf({}, 1)` | `TypeError: Object prototype may only be an Object or null: 1` | linked the primitive |
| `Object.setPrototypeOf(Object.freeze({}), {})` | `TypeError: #<Object> is not extensible` | re-linked the frozen object |
| `Object.keys/values/entries/getOwnPropertyNames/getOwnPropertySymbols/getOwnPropertyDescriptor/assign` on `null` | `TypeError: Cannot convert undefined or null to object` | empty result |
| `Symbol() + ''`, `` `${sym}` ``, `[sym].join()` | `TypeError: Cannot convert a Symbol value to a string` | rendered `Symbol(desc)` |
| `Symbol() + 1`, `Symbol() * 1`, `-Symbol()`, `Number(Symbol())` | `TypeError: Cannot convert a Symbol value to a number` | `NaN` / concatenation |
| `new (function*(){})()`, `new (async function(){})()`, `new (()=>{})()`, `new ({m(){}}).m()` | `TypeError: … is not a constructor` | ran the body, returned a half-built instance |

The checks do not over-reject, and that half is pinned too: a primitive is
object-coercible (`Object.keys(1)` is `[]`), a nullish SOURCE to `assign` is
skipped, re-setting the SAME prototype stays legal on a frozen object
(`Object.setPrototypeOf(Object.freeze({}), Object.prototype)`), and
`String(sym)`, `sym.toString()`, `sym.description` and a symbol PROPERTY KEY all
still work — those are the documented exceptions (22.1.1.1 step 2a).

### `AssertionError` carries node's whole property set

A failing assertion raised an internal `Name [CODE]: message` string, and the
error a `catch` received was synthesized from that string alone: `code`,
`message` and `stack` and nothing else. Everything a test runner reports —
`err.actual`, `err.expected`, `err.operator`, `err.generatedMessage` — was
`undefined`, and `err.constructor.name` said `Error` while `err.name` said
`AssertionError`. The failure now parks a real object in `host.exc`, so the
thrown VALUE is the object and the string stays only for the uncaught print.

Measured on node v26.7.0, `Object.keys(err)` is
`["generatedMessage","code","actual","expected","operator","diff"]` — in that
order — while `name`, `message` and `stack` are own but non-enumerable. The
`operator` is the METHOD name for the strict/deep forms (`strictEqual`,
`notStrictEqual`, `deepStrictEqual`, …) and the OPERATOR for the two loose ones
(`==`, `!=`); `ok(x)` reports `actual: x` against `expected: true` under `==`;
`fail`/`throws`/`doesNotThrow` report `operator` as their own method name with
`generatedMessage: false`. `diff` is `"simple"` for every failure form measured.

### Test-suite census (round 7 theme B)

101 test functions across the seven files in `tests/` were checked for a body
that can execute ZERO assertions. Two could, and both were in `tests/ffi.rs`:
`rust_block_exports_are_callable_across_all_v1_signatures` and
`rust_block_with_no_exports_errors` each `return`ed early when `rustc` was not
runnable, reporting PASS having asserted nothing. The guard is now a hard
`assert!`: `cargo test` built the binary under test with the very toolchain
being probed, so an absent compiler is a broken environment rather than a reason
to pass.

`tests/parity.rs` had a subtler form — the corpus is walked off disk, and an
empty `examples/` satisfies `files.len() == expected.len()` as `0 == 0` and then
loops zero times. It now asserts a floor on the corpus size. The parser helpers
in `name_registry.rs` and `opcode_ids.rs` already carried that floor
(`out.len() > 40`, `> 60`, `> 30`, `> 50`, `!out.is_empty()`), so those tests
could not go vacuous; `embed.rs`, `timers.rs` and `es_parity.rs` have no
conditional assertion paths at all.

## Still open — found in round 7

| gap | node v26.7.0 | node-js |
| --- | --- | --- |
| recursion depth before `RangeError` | 9901 for `function f(){f()}` | ~2400 in a debug build — the floor is a STACK BUDGET, not a frame count, so the number moves with frame size (release reaches far more). Both throw; only the depth differs |
| `new Array(4294967295)` | `4294967295` (holes are lazy) | killed at 10s — a dense `Vec` cannot hold 2^32-1 elements. The LENGTH is legal, so the spec check above lets it through; this is the documented dense-array model, not a missing validation |
| `JSON.stringify` of a 20 000-deep object | prints promptly | killed at 60s (5 000 deep completes) |
| `async function f(){ return f() }; f()` | `RangeError: Maximum call stack size exceeded` | hangs — each call starts a coroutine and returns a promise, so the recursion is an unbounded MICROTASK chain rather than stack growth, and the stack guard never sees it |
| `new g()` where `g` is a `function*` | message names the callee's SOURCE TEXT (`o.m is not a constructor`) | names it by function NAME (`m is not a constructor`) — the class, `.name` and catchability all match; node-js keeps no spans |
| `new URL('/x')` error own properties | `["code","input","message","stack"]` | `["code","message","stack"]` — no `input` |
| `class B extends A { constructor(){ this.x = 1 } }` | `ReferenceError: Must call super constructor in derived class before accessing 'this' …` | no error — `this` is bound before `super()` |
| `'use strict'; const c = 1; c = 2` | `TypeError: Assignment to constant variable.` | assignment succeeds |
| `structuredClone(function(){})` | `DOMException` / `DataCloneError`, `code: 25` | no error |
| `Number.prototype.toFixed.call({})` | `TypeError: Number.prototype.toFixed requires that 'this' be a Number` | `TypeError: toFixed is not a function` — right class, wrong message |
| `String.prototype.at.call(null)` | `TypeError: String.prototype.at called on null or undefined` | `TypeError: at is not a function` |
| `const {a} = null` | `TypeError: Cannot destructure property 'a' of 'null' as it is null.` | `TypeError: Cannot read properties of null (reading 'a')` |
| `eval('await 1')` | `SyntaxError: await is only valid in async functions …` | `SyntaxError: expected ';' but found Num(1.0) (line 1)` |

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
- **Well-known symbols and symbol-keyed properties.** `Symbol.toPrimitive`,
  `Symbol.toStringTag` and `Symbol.hasInstance` exist alongside
  `Symbol.iterator`/`asyncIterator` — and only those five, because those are the
  ones node-js acts on (a symbol that read back while the operator it names
  ignored it would be a silent fake). Their description is now the ECMAScript
  name, so
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

## FIXED — the parity sweep against node v26.7.0 (`JSON` hooks, `%d`/`%i`/`%f`, cycles, `Symbol.hasInstance`, `static {}`)

Each row was found by running the same source through `/opt/homebrew/bin/node`
v26.7.0 and the built `node-js` and byte-diffing stdout, stderr and exit status.
All are now byte-identical and are pinned by `tests/es_parity.rs` plus the
`parity-scripts/` corpus (`data/23_json_replacer.js`, `data/24_json_tojson.js`,
`data/25_math_bigint_iso_years.js`, `stdlib/27_format_numeric_directives.js`,
`objects/05_inspect_cycles.js`, `objects/06_has_instance.js`,
`lang/29_static_blocks.js`).

| case | node v26.7.0 | node-js before |
| --- | --- | --- |
| `JSON.stringify({a:1,b:2},(k,v)=>v*2)` | `{"a":2,"b":4}` | `{"a":1,"b":2}` — a callable second argument was ignored entirely |
| `JSON.stringify({a:1,b:2},(k,v)=>k==='b'?undefined:v)` | `{"a":1}` | `{"a":1,"b":2}` |
| `JSON.stringify(5,(k,v)=>v*2)` | `10` | `5` — the top-level `("", value)` call never happened |
| `JSON.stringify({a:{toJSON(k){return 'K:'+k}}})` | `{"a":"K:a"}` | `{"a":"K:undefined"}` — `toJSON` was called with no arguments |
| `JSON.stringify({toJSON(){return {toJSON(){return 1}}}})` | `{}` | `1` — `toJSON` was re-applied to its own result |
| `console.log(c)` for `c={a:1}; c.c=c` | `<ref *1> { a: 1, c: [Circular *1] }` | `{ a: 1, c: { a: 1, c: { a: 1, c: [Object] } } }` |
| `util.format('%d', 1.7)` | `1.7` | `1` — `%d` is `Number(x)`, not a truncation |
| `util.format('%i', '3.9abc')` | `3` | `NaN` — `%i` is `parseInt(x, 10)` |
| `util.format('%f', '1.5x')` | `1.5` | `NaN` — `%f` is `parseFloat(x)` |
| `util.format('%d', Infinity)` | `Infinity` | `9223372036854775807` — the truncation saturated `i64` |
| `util.format('%d', 10n)` | `10n` | `10` |
| `Number.MIN_VALUE` | `5e-324` | `2.2250738585072014e-308` — the smallest NORMAL double, not the smallest subnormal |
| `2 instanceof Even` for `class Even { static [Symbol.hasInstance](n){return n%2===0} }` | `true` | `false` — `Symbol.hasInstance` did not exist and `instanceof` never consulted it |
| `class A { static { A.tag = 1 } }` | runs | `SyntaxError: bad member key Punct("{")` — the whole script failed to parse |
| `Math.max(1n)` | `TypeError: Cannot convert a BigInt value to a number` | `1` — the argument reader took the BigInt's magnitude |
| `new Date(8.64e15).toISOString()` | `+275760-09-13T00:00:00.000Z` | `275760-09-13T00:00:00.000Z` |
| `new Date(Date.UTC(-1,0,1)).toISOString()` | `-000001-01-01T00:00:00.000Z` | `-001-01-01T00:00:00.000Z` |

Still open from the same sweep, each needing substrate that does not exist yet:

| case | node v26.7.0 | node-js |
| --- | --- | --- |
| `new DataView(buf)` | a view | `ReferenceError: DataView is not defined` (already listed above) |
| `console.log([,1,,2])` | `[ <1 empty item>, 1, <1 empty item>, 2 ]` | `[ undefined, 1, undefined, 2 ]` — the dense-array model has no HOLE, so `0 in [,1]` also reads `true` |
| `'café'.normalize('NFC').length` | `4` | `5` — `normalize` is the documented identity (no Unicode normalization tables) |

## Proxy

`Proxy` is implemented, with all thirteen traps and `Proxy.revocable`. It is a
real heap variant (`JsObj::Proxy`), not an object with hidden slots, because
`typeof`, callability and constructability all have to answer from the TARGET
while every property operation answers from the HANDLER — a shape no property
map can carry. `src/proxy.rs` is the single place the diversion happens; the
funnels the rest of the runtime already routed through call into it first.

| trap | reached through |
| --- | --- |
| `get` | `p.k`, `p[k]`, `Reflect.get`, a proxy used as a PROTOTYPE (the child is the trap's `receiver`) |
| `set` | `p.k = v`, `p[k] = v`, `Reflect.set` |
| `has` | `k in p`, `Reflect.has` |
| `deleteProperty` | `delete p.k`, `delete p[k]`, `Reflect.deleteProperty` |
| `ownKeys` | `Object.keys`/`values`/`entries`, `Object.getOwnPropertyNames`/`Symbols`, `Reflect.ownKeys`, `for-in`, object spread, `Object.assign`, `JSON.stringify` |
| `getOwnPropertyDescriptor` | `Object.getOwnPropertyDescriptor(s)`, `Object.prototype.hasOwnProperty`, and the enumerability filter in front of `Object.keys` / `for-in` |
| `defineProperty` | `Object.defineProperty`/`defineProperties`, `Reflect.defineProperty` |
| `getPrototypeOf` | `Object.getPrototypeOf`, `Reflect.getPrototypeOf`, `instanceof` |
| `setPrototypeOf` | `Object.setPrototypeOf`, `Reflect.setPrototypeOf` |
| `isExtensible` | `Object.isExtensible`, `Reflect.isExtensible` |
| `preventExtensions` | `Object.preventExtensions`, `Reflect.preventExtensions` |
| `apply` | `p(…)`, `p.call`/`.apply`/`.bind`, `Reflect.apply` |
| `construct` | `new p(…)`, `Reflect.construct`, `super(…)` from a class that extends the proxy |

`Reflect.get(target, key, receiver)` now honors its third argument — a getter
runs with the RECEIVER as `this`, which is what makes the standard
`get(t, k, r) { return Reflect.get(t, k, r) }` membrane forward correctly when
the proxy is a prototype.

Iteration deserves a note. `Array.prototype[Symbol.iterator]` is generic: it
reads `length` and then each index through `[[Get]]`. node-js models it as a
thunk BOUND to the array it was read off, which read through a proxy would walk
the target and ignore every answer the `get` trap gave. An array-backed proxy
still holding that default therefore takes an explicit length-driven walk, so
`[...new Proxy([1,2,3], { get: (t,k) => k === 'length' ? 2 : t[k] })]` is
`[1, 2]` as in Node. A user-installed `Symbol.iterator` is an ordinary function
and keeps the direct path.

The same "generic method modeled as a bound thunk" correction applies to
`Function.prototype.call`/`apply`/`bind`/`toString` and to the reflective
`Object.prototype` methods (`hasOwnProperty`, `propertyIsEnumerable`,
`isPrototypeOf`): read off a proxy they resolve to a thunk bound to the TARGET,
so each is re-dispatched against the proxy. `pf.call(1, 2)` therefore reaches the
`apply` trap, `p.hasOwnProperty(k)` reaches the descriptor trap, and
`String(new Proxy(function f(){}, {}))` is `function () { [native code] }` —
V8 refuses to expose a proxy's target through `Function.prototype.toString`. The
kind-specific `toString`/`valueOf`/`toLocaleString` are deliberately left on the
thunk, because they resolve by the target's kind: a proxy of an array
stringifies `1,2`, not `[object Object]`.

Two divergences remain, both cases where node-js is more permissive than Node:

| case | node v26.7.0 | node-js |
| --- | --- | --- |
| `new Proxy(new Map([['a',1]]), {}).get('a')` | `TypeError: Method Map.prototype.get called on incompatible receiver #<Map>` | `1` — a builtin method read through a proxy binds to the TARGET, so the internal-slot check Node performs on `this` never runs |
| `structuredClone(new Proxy({a:1}, {}))` | `DOMException [DataCloneError]` | `{a:1}` — `structuredClone` has no uncloneable-value rejection at all (functions and `WeakMap` clone silently too), and node-js has no `DOMException` |

The spec's trap-result INVARIANT checks are not implemented: 10.5.x throws when a
trap contradicts a non-configurable or non-extensible property of the target
(reporting a frozen own property as absent, say). node-js reports the trap's
answer as given. Every trap itself is real — the gap is the after-the-fact
consistency audit, and it is listed here rather than papered over.
