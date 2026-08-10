```text
███╗   ██╗ ██████╗ ██████╗ ███████╗         ██╗███████╗
████╗  ██║██╔═══██╗██╔══██╗██╔════╝         ██║██╔════╝
██╔██╗ ██║██║   ██║██║  ██║█████╗█████╗     ██║███████╗
██║╚██╗██║██║   ██║██║  ██║██╔══╝╚════╝██   ██║╚════██║
██║ ╚████║╚██████╔╝██████╔╝███████╗    ╚█████╔╝███████║
╚═╝  ╚═══╝ ╚═════╝ ╚═════╝ ╚══════╝     ╚════╝ ╚══════╝
```

![Rust](https://img.shields.io/badge/Rust-2021-05d9e8?style=flat-square)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://menketechnologies.github.io/node-js/)
[![Built on](https://img.shields.io/badge/built%20on-fusevm-8a2be2.svg)](https://github.com/MenkeTechnologies/fusevm)
![status](https://img.shields.io/badge/status-active%20%C2%B7%20in%20development-9b5de5?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-ff2a6d?style=flat-square)

### `[JAVASCRIPT, COMPILED TO BYTECODE — ON A SHARED CRANELIFT JIT]`

> *"V8 compiles JavaScript to its own bytecode and JITs it with its own engine.
> node-js lowers JavaScript to a shared machine that other languages already run
> on, and lets one Cranelift JIT compile the hot loops."*

**node-js** is JavaScript as a [`fusevm`](https://github.com/MenkeTechnologies/fusevm)
frontend — a lexer/parser and compiler that lowers JavaScript to `fusevm::Chunk`
bytecode running on fusevm's bytecode VM + Cranelift JIT, over a `JsHost` object
heap. There is no bespoke interpreter loop: node-js is a pure front end;
execution and codegen live in `fusevm` — the same engine behind
[`zshrs`](https://github.com/MenkeTechnologies/zshrs),
[`strykelang`](https://github.com/MenkeTechnologies/strykelang),
[`awkrs`](https://github.com/MenkeTechnologies/awkrs),
[`pythonrs`](https://github.com/MenkeTechnologies/pythonrs), and
[`rubylang`](https://github.com/MenkeTechnologies/rubylang).

The binary is `node`.

### [`Read the Docs`](https://menketechnologies.github.io/node-js/) &middot; [`Engineering Report`](https://menketechnologies.github.io/node-js/report.html) &middot; [`Builtin Reference`](https://menketechnologies.github.io/node-js/reference.html) &middot; [`fusevm`](https://github.com/MenkeTechnologies/fusevm)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Pipeline](#0x01-pipeline)
- [\[0x02\] Usage](#0x02-usage)
- [\[0x03\] Supported Today](#0x03-supported-today)
- [\[0x04\] Not Yet (Later Waves)](#0x04-not-yet-later-waves)
- [\[0x05\] Parity Harness & Fuzzer](#0x05-parity-harness--fuzzer)
- [\[0x06\] Build](#0x06-build)
- [\[0x07\] Documentation](#0x07-documentation)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

node-js keeps JavaScript the language and throws away V8's execution model. It
lexes and parses JavaScript to an AST, lowers the AST to `fusevm` bytecode, and
runs it on the shared bytecode VM with a Cranelift JIT. Arithmetic and
comparisons lower to native ops so the JIT can trace hot loops; JS-specific
behavior — truthiness, `==` coercion, `+` overloading, `ToInt32` bitwise wrap,
number formatting, the builtin objects — is served by the `JsHost` object heap
through fusevm's builtin dispatch and a strict numeric hook.

It carries no VM or JIT of its own. Bug fixes and JIT improvements in `fusevm`
land once and benefit every hosted frontend at the same time.

## [0x01] PIPELINE

```
source ──▶ lexer ──▶ parser ──▶ compiler ──▶ fusevm::Chunk ──▶ fusevm VM + JIT
              │         │           │                                  │
          tokens     JS AST    lower to bytecode              callbacks into JsHost
        (+ template  (funcs,   (native ops + CallBuiltin)     (builtins + numeric hook)
         re-lex)     arrows,
                     try/catch)
```

- **Primitives** (`number`, `boolean`, `null`, `undefined`) ride through the VM
  as native `fusevm::Value`s.
- **Objects, strings, and arrays** are heap objects in `JsHost`; they travel as
  `Value::Obj(u32)` handles into that heap, and property insertion order is
  preserved (observable in iteration and `JSON` round-trips).
- **Arithmetic** lowers to native fusevm ops so the JIT can trace hot loops; a
  strict **numeric hook** supplies JS coercion for the non-numeric operand cases
  (`+` string concat, `==` matrix, `ToInt32` for bitwise ops). An object operand
  goes through a real `ToPrimitive` first — which calls the user's
  `Symbol.toPrimitive`/`valueOf`/`toString`, so it runs before the host borrow
  the numeric hook takes. Everything JS-specific lowers to `CallBuiltin`
  handlers.

## [0x02] USAGE

```sh
node script.js                       # run a file
node -e 'console.log(1 + 1)'         # evaluate a one-liner
node -p '6 * 7'                      # evaluate and print the result
echo 'console.log(6 * 7)' | node     # read a script from stdin
node --tiers script.js               # run it, then report which fusevm tiers took it
```

Errors go to stderr in terse `node: <reason>` form; nothing else is printed. A
program that ran to completion exits with `process.exitCode` if it set one, and
the `beforeExit`/`exit` events fire as they do in Node.
Runnable `examples/*.js` ship with the crate.

## [0x03] SUPPORTED TODAY

A working core, grown outward from the sibling frontends. Implemented end-to-end
(see `examples/*.js` and `tests/parity.rs`):

- `var` / `let` / `const`; block scoping; expression and block statements.
- Full operator surface: arithmetic (`+ - * / % **`), string `+`, comparison
  (`== != === !== < > <= >=`), logical (`&& || !`), nullish `??`, bitwise
  (`& | ^ ~ << >> >>>`), `typeof` / `void` / `delete` / `instanceof` / `in`,
  conditional `?:`, sequence `,`, pre/post `++`/`--`, compound assignment.
- `if` / `else`, `while`, `do … while`, `for`, `for … in`, `for … of`,
  `switch`, `break`, `continue`, `return`, `throw`, `try` / `catch` / `finally`.
- `function` declarations and expressions, **arrow functions** (with `=>`
  lookahead detection), closures, recursion, `new`.
- Array and object literals, member (`a.b`) and index (`a[i]`) access, spread
  (`...`), **template literals** (`` `${...}` `` re-lexed from source).
- Builtin objects and methods on the `JsHost` heap: `console` (`log`), `Math`
  (`floor`/`ceil`/`round`/`trunc`/`abs`/`sign`/`max`/`min`/`pow`/`sqrt`/`cbrt`/
  `random`/`hypot`/`log`/`log2`/`log10`/`exp`/trig, `PI`/`E`), `JSON`
  (`stringify`/`parse`), `Object` (`keys`/`values`/`entries`/`hasOwnProperty`),
  `Array`, `Number` (`MAX_SAFE_INTEGER`/`EPSILON`/…), `String`, `Boolean`,
  `parseInt`/`parseFloat`/`isNaN`/`isFinite`, and a broad array/string method set
  (`map`/`filter`/`reduce`/`forEach`/`find`/`every`/`some`/`push`/`pop`/`slice`/
  `join`/`concat`/`includes`/`indexOf`/`flat`/`flatMap`/`reverse`/`fill`/`at`,
  `charAt`/`charCodeAt`/`padStart`/`padEnd`/`repeat`/`replace`/`replaceAll`/
  `startsWith`/`endsWith`, …).
- Strings are indexed by **UTF-16 code unit**, as JS specifies, so a
  supplementary-plane character counts as two: `"𝒳".length` is `2` and
  `"ab𝒳cd".indexOf("c")` is `4`. `[Symbol.iterator]` still yields code points
  (`[..."𝒳"]` is one element). `src/utf16.rs` is the single UTF-8 ⇄ UTF-16
  boundary; see BUGS.md for the one remaining gap (a value holding an unpaired
  surrogate, which a Rust `String` cannot represent). The same unit count drives
  relational comparison and the default `sort` order (an astral character sorts
  BELOW every BMP character from `U+E000` up), and the Buffer encodings defined
  over code units — `utf16le`/`ucs2` and the low byte each unit contributes to
  `latin1`/`ascii`.
- Annex B `escape`/`unescape` and ES2024
  `String.prototype.isWellFormed`/`toWellFormed`.
- `class` declarations and expressions: inheritance and `super`, static and
  instance fields, getters/setters, private `#` names, and a class body that
  evaluates in its own environment (so a static initializer can name its class).
- `async` / `await` and the microtask queue, generators and `yield` /
  `yield*`, async generators and `for await`, `Promise` (including
  `all`/`allSettled`/`race`/`any`).
- Destructuring patterns (array, object, nested, `...rest`) with defaults;
  default and rest parameters; labeled `break`/`continue`.
- `RegExp` (literals and constructor, named groups, the `String.prototype`
  regex methods), `Map` / `Set` / `WeakMap` / `WeakSet`, `Symbol`, `BigInt`,
  typed arrays and `Buffer`.
- CommonJS `require` and the Node standard library — see `BUGS.md` for the
  module-by-module coverage list and the honest not-implemented set.
- The persistent bytecode cache runs on EVERY invocation (schema-versioned, so
  an older cached script never replays incompatible bytecode), and AOT
  native-executable emission is on the CLI as `--build`.
- An LSP server (`--lsp`) and a DAP debug adapter (`--dap`) — source-line and
  function breakpoints, stepping, call stack, locals, and expression
  `evaluate` — are wired.
- **Running out of stack is a catchable error.** A JS call is a Rust recursion
  (each one builds a `fusevm::VM` on the stack), so the program runs on a
  dedicated deep-stack thread and every nested run checks the live stack pointer
  against the running stack's real bounds. Unbounded recursion — direct, through
  a recursive `valueOf`/`toString`, or inside a generator body on its own
  coroutine stack — raises `RangeError: Maximum call stack size exceeded`, the
  error V8 raises, rather than aborting the process. Depth is a byte budget, not
  a frame count, so it tracks the build's real frame size; BUGS.md records the
  measured numbers.

## [0x04] NOT YET (LATER WAVES)

ES modules: the static `import`/`export` forms do not parse, so every module
boundary has to go through CommonJS `require`. The file EXTENSION is not
consulted — a `.mjs` file holding only CommonJS-compatible code runs — so what
fails is module syntax, not the suffix. Dynamic `import()` does parse (it lexes
as an ordinary call) and fails at run time with
`ReferenceError: import is not defined`. `Proxy` and `Intl` are absent by design rather
than by omission — the reasoning for each is in `BUGS.md`, which also lists the
remaining behavioural divergences from the reference `node`.

## [0x05] PARITY HARNESS & FUZZER

Two differential tools check node-js against the reference `node`.

**`parity`** runs a fixed corpus through node-js and the reference `node`,
diffing stdout. It is a development tool — generating expectations needs `node`
on `PATH`, so CI never runs it; the frozen outputs live in
`tests/data/parity_expected.txt`, which `tests/parity.rs` replays with no `node`
installed.

**`parity-fuzz`** generates thousands of deterministic-output JS snippets and
diffs `node -e` against the reference `node -e`, delta-debugging every divergence
to a minimal repro. It is subprocess-only (never links the lib), std-only (no
`rand`), and needs `node` on `PATH`, so CI never runs it.

The two tools deliberately drive DIFFERENT entry points — the corpus runs each
case as a script FILE, the fuzzer through `-e` — because Node itself answers
differently at each (`__filename`, `module.id`, `process.argv`,
`process.execArgv`, top-level `this`). BUGS.md tabulates the full set; a case
that touches any of it is measuring one entry point, not "node".

```sh
cargo build --bin parity --bin parity-fuzz
./target/debug/parity                          # run the corpus vs reference node
./target/debug/parity-fuzz --count 5000        # fuzz 5000 cases
./target/debug/parity-fuzz --once --seed 1234  # replay one case, show both sides
```

All three compare the exit STATUS exactly rather than as zero-vs-nonzero, and
run both children with `TZ=UTC` and `LANG=LC_ALL=en_US.UTF-8` pinned rather than
inherited: reference `node` is not locale- or TZ-invariant, and a status
collapsed to a boolean cannot see `process.exitCode`, which prints nothing.
`parity --bless` re-records the frozen snapshot from the REFERENCE process — the
only supported way to regenerate it, and never from node-js's own output.

A third harness, `parity-scripts/run.sh`, byte-compares every
`parity-scripts/**/*.js` file against the reference `node` (stdout AND exit
status) and prints the pass rate:

```sh
bash parity-scripts/run.sh      # byte-parity rate over the whole corpus
bash parity-scripts/run.sh -v   # plus a diff for each divergence
```

Fuzz generators are biased toward where a JS frontend is likely to disagree with
the reference: float representation and the exponential-notation threshold,
`ToInt32` bitwise wrap, the `==` coercion matrix, `+` coercion, string/array
methods, `toFixed`/`toPrecision` rounding, JSON round-trips and parse-error
messages, property descriptors and the enumeration surface that depends on them,
`freeze`/`seal` write and `delete` outcomes, builtin identity and prototype-chain
reads, `structuredClone`'s reference graph, error own-property shape, abrupt
completions (`unwind`), and promise-resolution / async-iteration microtask
ordering (`thenable`). Select one with `--mode <name>`.

The run summary reports four counts next to the divergence total: **ref timeout**
(the reference timed out, so the case is skipped entirely), **ref failed** (the
reference exited non-zero), **ref silent** (the reference printed nothing on
stdout) and **ref inert** (both — the only condition under which a case observed
nothing at all). A mode scoring zero divergences while **ref inert** is high is
comparing nothing; `ref failed` on its own is not that signal any more, since
the exit code is itself a compared value and the `exit` mode consists of
programs that print nothing and exit non-zero deliberately.

## [0x06] BUILD

```sh
cargo build
cargo test
```

node-js is a standalone crate (an explicit empty `[workspace]` stops cargo
walking up to the meta parent). `fusevm` is pulled from crates.io with the `jit`,
`jit-disk-cache`, `aot`, and `ffi` features.

## [0x07] DOCUMENTATION

- **Docs hub** — <https://menketechnologies.github.io/node-js/>
- **Builtin reference** — <https://menketechnologies.github.io/node-js/reference.html>
- **Engineering report** — <https://menketechnologies.github.io/node-js/report.html>
- **fusevm** — <https://github.com/MenkeTechnologies/fusevm> (the shared VM)
- **Source** — <https://github.com/MenkeTechnologies/node-js>

## [0xFF] LICENSE

MIT — free and open source. See [LICENSE](LICENSE).
