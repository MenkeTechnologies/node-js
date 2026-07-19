# node-js — known gaps and unimplemented behavior

node-js is JavaScript lowered to fusevm (bytecode VM + Cranelift JIT), with a
JsHost object heap. It runs a real subset of JavaScript correctly, verified
byte-for-byte against system `node` on the example corpus (`tests/parity.rs`) and
via the differential fuzzer (`parity-fuzz`, 20000 cases clean against
`node v26.5.0`). This file is the honest list of what is **not** yet covered, so
nobody mistakes a gap for a bug fixed.

The `parity-fuzz` generator deliberately stays within the implemented surface:
its contract is "find real bugs in shipped features," so it does not emit the
constructs below. Each is a genuine gap, not something the harness hides.

## Not implemented (parse/compile-time error, no silent wrong answer)

These are absent from the lexer/parser/compiler; a program using them fails
loudly rather than producing a wrong result. The fuzzer does not generate them.

- **`class`** declarations/expressions, `extends`, `super`, `static`, private
  `#fields`. No prototype-chain modelling (`instanceof` always returns `false`).
- **Generators** — `function*` / `yield` / `yield*`.
- **`async` / `await`** and Promises — no event loop.
- **Regular-expression literals** (`/pat/flags`) and the `RegExp` object. String
  `replace`/`replaceAll`/`split` take only string arguments, not regexes.
- **`Map` / `Set` / `WeakMap` / `WeakSet` / `Symbol`.**
- **`BigInt`** — the `10n` literal and bigint arithmetic. All numbers are IEEE-754
  `f64` (JS's single number type for non-bigint values).

## Partial / simplified semantics (runs, but not byte-identical to node in edge
cases the fuzzer is scoped away from)

- **`util.inspect` multi-line array grouping.** `console.log` of an array with
  **more than 6 elements** is where node switches to its multi-column, multi-line
  layout (`groupArrayElements`: computes column widths from a bias heuristic and
  wraps). node-js prints every array on a single line
  (`[ 1, 2, 3, 4, 5, 6, 7 ]`), which matches node exactly for arrays of ≤6
  elements but not beyond. The grouping algorithm is deterministic but intricate;
  until it is ported, the fuzzer keeps generated arrays at ≤6 elements so it
  exercises the (correct) single-line regime rather than flooding on this one
  formatting gap. Nested-object/array width-based line breaking (breakLength 80)
  is likewise not modelled.

- **`Number.prototype.toString(radix)` for a non-integer receiver.** The integer
  part is converted correctly for any radix 2..36 (`(255).toString(16) === "ff"`).
  The **fractional** part is currently dropped (`(3.14).toString(2)` yields
  `"11"`, not node's full `"11.001000111101011100001010001111010111000010100011111"`).
  For power-of-two radices the fraction terminates exactly and could be emitted
  faithfully; for other radices node/V8 emits a bounded round-trip expansion whose
  exact digit count is engine-specific. The fuzzer's `num` mode uses integer
  receivers for `toString(radix)` until this is implemented.

## Fixed since the initial parity sweep (previously divergences, now correct)

Recorded so the same gaps are not "re-discovered" as regressions. All verified
against `node v26.5.0`:

- **Number → string exponential threshold.** `fmt_number` now follows the
  ECMAScript Number::toString layout: `(1e21).toString() === "1e+21"`,
  `(1e-7).toString() === "1e-7"`, `(1e100) === "1e+100"` (Rust's `Display` would
  print the fully-expanded decimal). Fixed-vs-exponential boundary at `n > 21` /
  `n ≤ -6`.
- **`x / 0` division.** JS requires `1/0 === Infinity`, `-1/0 === -Infinity`,
  `0/0 === NaN`; fusevm's native `Op::Div` returns `Undef` for a zero divisor, so
  `/` is lowered to a node-js builtin (fusevm's documented pattern for a frontend
  whose `/` differs) that applies IEEE-754 semantics.
- **`+` operand coercion (`ToPrimitive`).** `[1,2,3] + 3 === "1,2,33"`,
  `{} + [] === "[object Object]"` — arrays/objects coerce to their string
  `toString` before `+`, not to `NaN`.
- **`==` loose equality.** Follows Abstract Equality with `ToPrimitive`:
  `[0] == "0"` is `true` but `[0] == ""` is `false` (string compare, never a
  number coercion of the object).
- **`Number.prototype.toFixed`** — rounds half away from zero on the exact value
  (`(2.5).toFixed(0) === "3"`), preserves the sign of a zero result
  (`(-0.4).toFixed(0) === "-0"`), and keeps full precision for large magnitudes
  (`(9.999999e20).toFixed(4)`).
- **`Number.prototype.toPrecision`** — significant-digit rounding half away from
  zero with the exponential switch at `e < -6` or `e ≥ p`
  (`(2.5).toPrecision(1) === "3"`, `(999.5).toPrecision(3) === "1.00e+3"`).
- **`Math.hypot`** — scaled algorithm (`max · √Σ(xᵢ/max)²`) matching V8's last-ULP
  result, not the naive `√Σxᵢ²`.
- **`Math.round`** preserves negative zero (`Math.round(-0.4) === -0`).
- **`String.prototype.slice`/`substr`** — reversed bounds yield the empty string
  (`"World".slice(2, 1) === ""`); `substr` handles a negative start.
- **`parseFloat`** parses `Infinity` / `-Infinity`.
