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

### [`Read the Docs`](https://menketechnologies.github.io/node-js/) &middot; [`Engineering Report`](https://menketechnologies.github.io/node-js/report.html) &middot; [`fusevm`](https://github.com/MenkeTechnologies/fusevm)

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
  (`+` string concat, `==` matrix, `ToInt32` for bitwise ops). Everything
  JS-specific lowers to `CallBuiltin` handlers.

## [0x02] USAGE

```sh
node script.js                       # run a file
node -e 'console.log(1 + 1)'         # evaluate a one-liner
echo 'console.log(6 * 7)' | node     # read a script from stdin
```

Errors go to stderr in terse `node: <reason>` form; nothing else is printed.
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

## [0x04] NOT YET (LATER WAVES)

`class` (declarations, inheritance, methods), `async` / `await`, generators
(`yield`), ES modules (`import` / `export`), destructuring patterns, default and
rest parameters, labeled statements, `RegExp`, `Map` / `Set` / `Promise`, and the
Node.js standard library (`fs`, `path`, `process`, `require`, event loop). The
crate is built as a `staticlib` with fusevm's `aot` and `jit-disk-cache` features
enabled (mirroring `pythonrs`), but **AOT native-executable emission and the
persistent bytecode cache are not yet exposed on the CLI**. An LSP server
(`--lsp`) and a DAP debug adapter (`--dap`) — source-line and function
breakpoints, stepping, call stack, locals, and expression `evaluate` — are wired.

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

```sh
cargo build --bin parity --bin parity-fuzz
./target/debug/parity                          # run the corpus vs reference node
./target/debug/parity-fuzz --count 5000        # fuzz 5000 cases
./target/debug/parity-fuzz --once --seed 1234  # replay one case, show both sides
```

Fuzz generators are biased toward where a JS frontend is likely to disagree with
the reference: float representation and the exponential-notation threshold,
`ToInt32` bitwise wrap, the `==` coercion matrix, `+` coercion, string/array
methods, `toFixed`/`toPrecision` rounding, and JSON round-trips.

## [0x06] BUILD

```sh
cargo build
cargo test
```

node-js is a standalone crate (an explicit empty `[workspace]` stops cargo
walking up to the meta parent). `fusevm` is pulled from crates.io with the `jit`,
`jit-disk-cache`, and `aot` features.

## [0x07] DOCUMENTATION

- **Docs hub** — <https://menketechnologies.github.io/node-js/>
- **Engineering report** — <https://menketechnologies.github.io/node-js/report.html>
- **fusevm** — <https://github.com/MenkeTechnologies/fusevm> (the shared VM)
- **Source** — <https://github.com/MenkeTechnologies/node-js>

## [0xFF] LICENSE

MIT — free and open source. See [LICENSE](LICENSE).
