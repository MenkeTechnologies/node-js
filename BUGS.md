# node-js — known gaps and unimplemented behavior

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
dedicated fuzzer modes (`class`, `generator`, `mapset`, `proto`, `async`) that
track the surface:

- **ES6 classes** — `class`/`extends`/`super(...)`/`super.method()`, constructor,
  instance + static methods, instance + static fields, `get`/`set` accessors,
  computed method names, private `#fields`/`#methods`, `new.target`, constructor
  object-return, static inheritance down the constructor chain, `class extends
  Error`.
- **Prototype chain** — `[[Prototype]]` delegation for property lookup;
  `Object.getPrototypeOf`/`setPrototypeOf`/`create`; `obj.__proto__` (read + the
  literal `{ __proto__: x }` form); `defineProperty`/`getOwnPropertyDescriptor`.
- **`instanceof`** (walks the chain; structural for builtin `Array`/`Object`/
  `Function`), **`in`** and **`hasOwnProperty`** respecting the chain.
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
- **Promises + async/await + event loop** — `new Promise`, `.then`/`.catch`/
  `.finally`, `Promise.resolve`/`reject`/`all`/`allSettled`/`race`/`any`; `async`
  functions/arrows/methods, `await`, rejection-as-throw; a host-driven loop
  draining `process.nextTick` → promise microtasks → timers
  (`setTimeout`/`setInterval`/`setImmediate`, `queueMicrotask`), Node ordering.

## Not implemented (parse/compile-time error, no silent wrong answer)

These are absent from the lexer/parser/compiler; a program using them fails
loudly rather than producing a wrong result. The fuzzer does not generate them.

- **Regular-expression literals** (`/pat/flags`) and the `RegExp` object. String
  `replace`/`replaceAll`/`split` take only string arguments, not regexes.
- **`BigInt`** — the `10n` literal and bigint arithmetic. All numbers are IEEE-754
  `f64` (JS's single number type for non-bigint values).
- **Async generators / async iteration** — `async function*` parses (the `async`
  prefix is accepted) but `for await (…)` and the async-iterator protocol are not
  modeled; a plain generator is produced.
- **Tagged template literals** (`` tag`...` ``) and `String.raw`.
- **Labeled statements** (`outer: for (...)` with `break outer`). Labels are
  parsed after `break`/`continue` but not bound to a target.

## Partial / simplified semantics (runs, but not byte-identical to node in edge
cases the fuzzer is scoped away from)

- **`util.inspect` multi-line array grouping.** `console.log` of an array with
  **more than 6 elements** is where node switches to its multi-column, multi-line
  layout (`groupArrayElements`). node-js prints every array on a single line,
  which matches node exactly for arrays of ≤6 elements but not beyond. Until the
  grouping algorithm is ported, the fuzzer keeps generated arrays at ≤6 elements.
  Nested-object/array width-based line breaking (breakLength 80) is likewise not
  modelled.

- **`generator.return()` / `.throw()` do not run `finally`.** Calling `.return(v)`
  or `.throw(e)` on a suspended generator marks it done and reports the completion,
  but does NOT resume the coroutine to execute a pending `try { … } finally { … }`
  cleanup block (node runs the finalizer). The common `for-of` + `break` early-exit
  path is unaffected (it simply abandons the iterator). The fuzzer does not
  generate `.return()`/`.throw()` with a `finally`.

- **`Number.prototype.toString(radix)` for a non-integer receiver.** The integer
  part is converted correctly for any radix 2..36; the **fractional** part is
  dropped. The fuzzer's `num` mode uses integer receivers for `toString(radix)`.

- **`Object.create(null)` vs an ordinary object under `instanceof Object`.** A
  null-prototype object is reported as `instanceof Object` (node reports `false`).
  Our model can't distinguish "no explicit prototype" from "explicit null
  prototype", so a bare object literal and `Object.create(null)` both read as
  Object instances. Not exercised by the fuzzer.

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
