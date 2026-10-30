//! ES module parity: static `import`/`export`, live bindings, cycles,
//! namespaces, top-level `await`, CommonJS interop, resolution and the module
//! early errors.
//!
//! Every expected output was captured from system `node v26.12.1` running the
//! same files. Each test writes its module graph into a fresh temporary
//! directory and runs the built binary there, so no Node install is needed in
//! CI. Outputs never print absolute paths, which differ per machine.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Write `files` (relative path, contents) under a fresh directory and run the
/// binary there with `args`, feeding `stdin` when given.
fn run_with(files: &[(&str, &str)], args: &[&str], stdin: Option<&str>) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, src) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        std::fs::write(&path, src).expect("write module");
    }
    run_in(dir.path(), args, stdin)
}

fn run_in(dir: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    // `ESM_REFERENCE_NODE=/path/to/node` runs the same files through a
    // reference node instead, which is how the expectations are re-captured.
    let bin =
        std::env::var_os("ESM_REFERENCE_NODE").unwrap_or_else(|| env!("CARGO_BIN_EXE_node").into());
    let mut cmd = Command::new(bin);
    cmd.args(args).current_dir(dir).stdin(Stdio::piped());
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn node binary");
    {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin pipe");
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).expect("write stdin");
        }
    }
    child.wait_with_output().expect("wait for node binary")
}

/// [`run_with`] that must succeed; returns trimmed stdout.
fn stdout_of(files: &[(&str, &str)], args: &[&str]) -> String {
    let out = run_with(files, args, None);
    assert!(
        out.status.success(),
        "program failed ({}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn assert_lines(actual: &str, expected: &str) {
    let expected = expected.trim_matches('\n');
    assert_eq!(
        actual, expected,
        "\n--- actual ---\n{actual}\n--- expected ---\n{expected}"
    );
}

const LIB: &str = r#"export let count = 0;
export function inc() { count++; return count; }
export const obj = { k: 1 };
export default function () { return 'dflt'; }
export class Named {}
let hidden = 'h';
export { hidden as shown, hidden as default2 };
export * from './sub.mjs';
export * as subns from './sub.mjs';
export { subA as renamedA } from './sub.mjs';
console.log('lib evaluated');
"#;

const SUB: &str = r#"export const subA = 'A';
export const subB = 'B';
export default 'subdefault';
console.log('sub evaluated');
"#;

/// Imports are live views of the exporting module's bindings, and a module
/// namespace object is the sealed, null-prototype, sorted exotic object of
/// 10.4.6 — including its refusals, which V8 words its own way.
#[test]
fn live_bindings_and_the_namespace_object() {
    let main = r#"import dflt, { count, inc, obj, Named, shown, subA, subns, renamedA } from './lib.mjs';
import * as lib from './lib.mjs';
console.log('main start');
console.log(dflt(), dflt.name, count, inc(), count, obj, Named.name, shown, subA, renamedA);
console.log(lib.count, subns === lib.subns, lib.subns.subB);
console.log(lib);
console.log(Object.keys(lib), lib[Symbol.toStringTag], Object.getPrototypeOf(lib));
console.log(Object.isFrozen(lib), Object.isSealed(lib), Object.isExtensible(lib));
console.log(Object.getOwnPropertyDescriptor(lib, 'count'), 'count' in lib, 'nope' in lib);
console.log(Object.prototype.toString.call(lib), Reflect.set(lib, 'count', 9), Reflect.deleteProperty(lib, 'nope'));
try { lib.count = 5 } catch (e) { console.log(e.constructor.name, e.message) }
try { delete lib.count } catch (e) { console.log(e.constructor.name, e.message) }
try { Object.defineProperty(lib, 'count', { value: 2 }) } catch (e) { console.log(e.constructor.name, e.message) }
try { count = 1 } catch (e) { console.log(e.constructor.name, e.message) }
try { count++ } catch (e) { console.log(e.constructor.name, e.message) }
console.log(count, JSON.stringify(lib.obj));
"#;
    let out = stdout_of(
        &[("lib.mjs", LIB), ("sub.mjs", SUB), ("main.mjs", main)],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
sub evaluated
lib evaluated
main start
dflt default 0 1 1 { k: 1 } Named h A A
1 true B
[Module: null prototype] {
  Named: [class Named],
  count: 1,
  default: [Function: default],
  default2: 'h',
  inc: [Function: inc],
  obj: { k: 1 },
  renamedA: 'A',
  shown: 'h',
  subA: 'A',
  subB: 'B',
  subns: [Module: null prototype] {
    default: 'subdefault',
    subA: 'A',
    subB: 'B'
  }
}
[
  'Named',    'count',
  'default',  'default2',
  'inc',      'obj',
  'renamedA', 'shown',
  'subA',     'subB',
  'subns'
] Module null
false true false
{ value: 1, writable: true, enumerable: true, configurable: false } true false
[object Module] false true
TypeError Cannot assign to property 'count' of [object Module]
TypeError Cannot delete property 'count' of [object Module]
TypeError Cannot redefine property: count
TypeError Assignment to constant variable.
TypeError Assignment to constant variable.
1 {"k":1}
"#,
    );
}

/// An import cycle: the module evaluated first already sees the other's
/// hoisted function declarations (including an `export default function`),
/// while its `const` bindings are still in their dead zone — through a direct
/// read, `typeof`, and a call that reads one.
#[test]
fn cycles_see_hoisted_functions_and_the_dead_zone() {
    let a = r#"import { b, callA } from './b.mjs';
import selfDefault, * as self from './a.mjs';
export const a = 'a';
export function fa() { return 'fa'; }
export default function named() { return 'self default' }
console.log('a sees', b, callA(), selfDefault(), Object.keys(self));
"#;
    let b = r#"import aDefault, { a, fa } from './a.mjs';
export const b = 'b';
export function callA() { return a; }
console.log('b sees hoisted', fa(), aDefault.name, aDefault());
try { console.log(a) } catch (e) { console.log(e.constructor.name, e.message) }
try { typeof a } catch (e) { console.log('typeof', e.constructor.name) }
try { callA() } catch (e) { console.log('via function', e.constructor.name, e.message) }
"#;
    let out = stdout_of(&[("a.mjs", a), ("b.mjs", b)], &["a.mjs"]);
    assert_lines(
        &out,
        r#"
b sees hoisted fa named self default
ReferenceError Cannot access 'a' before initialization
typeof ReferenceError
via function ReferenceError Cannot access 'a' before initialization
a sees b a self default [ 'a', 'default', 'fa' ]
"#,
    );
}

/// `export default` names an anonymous function or class "default"; an
/// expression is bound as is.
#[test]
fn default_exports_and_their_names() {
    let main = r#"import d from './anon_class.mjs'; import e from './anon_expr.mjs'; import f from './anon_arrow.mjs';
import g from './anon_fn.mjs'; import h, { named as n2 } from './named_default.mjs';
console.log(d.name, e, f.name, f(), g.name, h.name, n2 === h);
"#;
    let out = stdout_of(
        &[
            ("main.mjs", main),
            ("anon_class.mjs", "export default class {}\n"),
            ("anon_expr.mjs", "export default 1 + 2;\n"),
            ("anon_arrow.mjs", "export default () => 9;\n"),
            ("anon_fn.mjs", "export default async function () {}\n"),
            (
                "named_default.mjs",
                "export default function named() {}; export { named };\n",
            ),
        ],
        &["main.mjs"],
    );
    assert_lines(&out, "default 3 default 9 default named true");
}

/// Importing CommonJS: `default` is `module.exports`, the named exports are
/// the ones node's cjs-module-lexer detects statically (assignments,
/// `defineProperty`, an object literal up to its first non-identifier value,
/// a `require` re-export), and `"module.exports"` is always present.
/// `require()` of an ES module returns its namespace — with `__esModule`
/// added when it has a default export — or its `"module.exports"` export, and
/// refuses a graph with top-level `await`.
#[test]
fn commonjs_interop_both_ways() {
    let main = r#"import cjs, { named, fn, other, defined } from './cjs.cjs';
import * as litns from './lit.cjs';
import * as reexp from './reexp.cjs';
import hello, * as fnns from './fn.cjs';
import { createRequire } from 'node:module';
console.log(cjs, named, fn(), other, defined);
console.log(Object.keys(litns), Object.keys(reexp), reexp.default === cjs);
console.log(hello(), fnns.default === hello, fnns.extra, Object.keys(fnns));
const require = createRequire(import.meta.url);
console.log(require('./cjs.cjs') === cjs);
const viaRequire = require('./esm.mjs');
console.log(viaRequire, viaRequire.__esModule, require('./esm.mjs') === viaRequire);
console.log(require('./exportsname.mjs'), require('./nodefault.mjs'));
try { require('./tla.mjs') } catch (e) { console.log(e.code) }
try { await import('./badnamed.mjs') } catch (e) { console.log(e.constructor.name + ': ' + e.message) }
"#;
    let cjs = r#"exports.named = 'n';
exports.fn = () => 'f';
module.exports.other = 2;
Object.defineProperty(exports, 'defined', { enumerable: true, value: 'd' });
console.log('cjs evaluated');
"#;
    let lit = r#"const a = 1, b = 2, e = 5;
module.exports = { a, bee: b, 'quoted': e, ...{ spread: 1 }, d: 1, after: a };
"#;
    let out = stdout_of(
        &[
            ("main.mjs", main),
            ("cjs.cjs", cjs),
            ("lit.cjs", lit),
            ("reexp.cjs", "module.exports = require('./cjs.cjs');\n"),
            (
                "fn.cjs",
                "module.exports = function hello() { return 'hello' }; module.exports.extra = 'e';\n",
            ),
            ("esm.mjs", "export const x = 1; export default 'd';\n"),
            (
                "exportsname.mjs",
                "export const z = 1; const v = { hello: 'world' }; export { v as 'module.exports' };\n",
            ),
            ("nodefault.mjs", "export const only = 1;\n"),
            ("tla.mjs", "await 1; export const t = 1;\n"),
            ("badnamed.mjs", "import { missing } from './cjs.cjs';\n"),
        ],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
cjs evaluated
{ named: 'n', fn: [Function (anonymous)], other: 2, defined: 'd' } n f 2 d
[ 'a', 'bee', 'default', 'module.exports', 'quoted' ] [ 'default', 'defined', 'fn', 'module.exports', 'named', 'other' ] true
hello true e [ 'default', 'extra', 'module.exports' ]
true
[Module: null prototype] { __esModule: true, default: 'd', x: 1 } true true
{ hello: 'world' } [Module: null prototype] { only: 1 }
ERR_REQUIRE_ASYNC_MODULE
SyntaxError: Named export 'missing' not found. The requested module './cjs.cjs' is a CommonJS module, which may not support all module.exports as named exports.
CommonJS modules can always be imported via the default export, for example using:

import pkg from './cjs.cjs';
const { missing } = pkg;
"#,
    );
}

/// Top-level `await`: an asynchronous dependency finishes before the modules
/// that wait on it run, in post-order; evaluation starts in a promise job, so
/// a dependency's microtask runs before its `nextTick`.
#[test]
fn top_level_await_and_evaluation_order() {
    let main = r#"import { log } from './slow.mjs';
import './fast.mjs';
import './dep_order.mjs';
log.push('main');
console.log(log.join(' > '));
const late = await Promise.all([1, 2].map(async (n) => { await null; return n * 2 }));
console.log(late);
for await (const v of (async function* () { yield 'a'; yield 'b' })()) console.log('for await', v);
"#;
    let slow = r#"export const log = [];
log.push('slow start');
await new Promise((r) => setTimeout(r, 20));
log.push('slow end');
"#;
    let micro = r#"Promise.resolve().then(() => console.log('dep microtask'));
process.nextTick(() => console.log('dep tick'));
console.log('dep body');
"#;
    let out = stdout_of(
        &[
            ("main.mjs", main),
            ("slow.mjs", slow),
            (
                "fast.mjs",
                "import { log } from './slow.mjs';\nlog.push('fast (after slow)');\n",
            ),
            (
                "dep_order.mjs",
                "import './micro_dep.mjs';\nconsole.log('dep_order body');\n",
            ),
            ("micro_dep.mjs", micro),
        ],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
dep body
dep_order body
dep microtask
dep tick
slow start > slow end > fast (after slow) > main
[ 2, 4 ]
for await a
for await b
"#,
    );
}

/// The unsettled top-level `await` warning names each module of the entry's
/// graph that is stalled in its OWN body — not the entry, and not a module
/// that only waits on a dependency — with node's source arrow under that
/// module's `await` and the three blank lines behind it. Captured from node
/// v26.12.1 with the directory prefix removed.
#[test]
fn unsettled_await_warning_points_at_the_stalled_await() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, src) in [
        (
            "main.mjs",
            "import \"./a.mjs\";\nimport \"./b.mjs\";\nawait 1;\n",
        ),
        (
            "a.mjs",
            "const x = 1;\n  const v = await  new Promise(() => {});\n",
        ),
        (
            "b.mjs",
            "import \"./a.mjs\";\nexport let y = await new Promise(() => {}).then(x => x);\n",
        ),
    ] {
        std::fs::write(dir.path().join(name), src).expect("write module");
    }
    let out = run_in(dir.path(), &["main.mjs"], None);
    assert_eq!(out.status.code(), Some(13));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let head = "Warning: Detected unsettled top-level await at file://";
    let rest = stderr
        .strip_prefix(head)
        .unwrap_or_else(|| panic!("{stderr}"));
    let rest = &rest[rest.find("/a.mjs:").unwrap_or_else(|| panic!("{stderr}"))..];
    assert_eq!(
        rest,
        "/a.mjs:2\n  const v = await  new Promise(() => {});\n            ^\n\n\n\n"
    );
}

/// The exit status of an entry module that never finishes its top-level
/// `await` is 13 (unless the program set one), with node's warning; one that
/// throws or rejects exits 1 after printing what ran first.
#[test]
fn entry_module_exit_statuses() {
    let files = [
        (
            "unsettled.mjs",
            "console.log('before'); await new Promise(() => {}); console.log('never');\n",
        ),
        (
            "unsettled_code.mjs",
            "process.exitCode = 3; await new Promise(() => {});\n",
        ),
        (
            "reject.mjs",
            "await Promise.reject(new TypeError('rejected'));\n",
        ),
        (
            "throw.mjs",
            "console.log('before'); throw new Error('boom');\n",
        ),
    ];
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, src) in files {
        std::fs::write(dir.path().join(name), src).expect("write module");
    }
    let unsettled = run_in(dir.path(), &["unsettled.mjs"], None);
    assert_eq!(unsettled.status.code(), Some(13));
    assert_eq!(String::from_utf8_lossy(&unsettled.stdout), "before\n");
    let warning = String::from_utf8_lossy(&unsettled.stderr);
    assert!(
        warning.starts_with("Warning: Detected unsettled top-level await at file://")
            && warning.contains("/unsettled.mjs"),
        "stderr: {warning}"
    );
    assert_eq!(
        run_in(dir.path(), &["unsettled_code.mjs"], None)
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        run_in(dir.path(), &["reject.mjs"], None).status.code(),
        Some(1)
    );
    let thrown = run_in(dir.path(), &["throw.mjs"], None);
    assert_eq!(thrown.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&thrown.stdout), "before\n");
    assert!(String::from_utf8_lossy(&thrown.stderr).contains("boom"));
}

/// Which files are modules: `.mjs`, a `"type": "module"` package scope, or —
/// for a `.js` file with no declared type, `-e` and stdin — module syntax
/// that makes the CommonJS parse fail (node's syntax detection). A module sees
/// none of the CommonJS wrapper names, and `this` is `undefined`.
#[test]
fn module_format_detection() {
    let files = [
        (
            "detect.js",
            "import { join } from 'node:path';\nconsole.log('detected', join('a', 'b'), typeof require, typeof module, typeof __filename, this);\n",
        ),
        (
            "tla_only.js",
            "console.log('tla', typeof require, typeof module); console.log(await Promise.resolve(1));\n",
        ),
        (
            "plain.js",
            "console.log('plain cjs', typeof require, this === module.exports);\n",
        ),
        ("typemod/package.json", "{ \"type\": \"module\" }\n"),
        (
            "typemod/m.js",
            "console.log('type module', typeof require, import.meta.url.endsWith('/typemod/m.js'));\n",
        ),
        ("typecjs/package.json", "{ \"type\": \"commonjs\" }\n"),
        ("typecjs/c.js", "console.log('type commonjs', typeof require);\n"),
        (
            "req_detect.cjs",
            "const m = require('./typemod/m.js'); console.log(Object.keys(m)); require('./detect.js');\n",
        ),
    ];
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, src) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        std::fs::write(path, src).expect("write module");
    }
    let run = |args: &[&str], stdin: Option<&str>| {
        let out = run_in(dir.path(), args, stdin);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    };
    assert_eq!(
        run(&["detect.js"], None),
        "detected a/b undefined undefined undefined undefined"
    );
    assert_eq!(run(&["tla_only.js"], None), "tla undefined undefined\n1");
    assert_eq!(run(&["plain.js"], None), "plain cjs function true");
    assert_eq!(run(&["typemod/m.js"], None), "type module undefined true");
    assert_eq!(run(&["typecjs/c.js"], None), "type commonjs function");
    assert_eq!(
        run(&["req_detect.cjs"], None),
        "type module undefined true\n[]\ndetected a/b undefined undefined undefined undefined"
    );
    assert_eq!(
        run(
            &[
                "-e",
                "import { sep } from 'node:path'; console.log(sep, import.meta.url.endsWith('/[eval1]'), typeof require)"
            ],
            None
        ),
        "/ true undefined"
    );
    assert_eq!(
        run(
            &[],
            Some("export {}; console.log(import.meta.main, await 2)")
        ),
        "false 2"
    );
}

/// Bare specifiers through `node_modules`: `"exports"` conditions (`import`
/// over `require`), subpath patterns, a subpath the package does not export,
/// legacy `"main"`; JSON modules need `with { type: "json" }`; `import()` and
/// `import.meta.resolve` use the same resolver.
#[test]
fn package_resolution_and_json_modules() {
    let main = r#"import pkg, { kind } from 'pkg';
import { feat } from 'pkg/feature/x';
import legacy, { c } from 'legacy';
import data from './data.json' with { type: 'json' };
console.log(pkg(), kind, feat, legacy, c, data);
console.log(import.meta.resolve('pkg').endsWith('/node_modules/pkg/lib/index.mjs'), import.meta.resolve('node:fs'));
try { await import('pkg/nope') } catch (e) { console.log(e.code) }
try { await import('nopkg') } catch (e) { console.log(e.code, e.message.startsWith("Cannot find package 'nopkg' imported from ")) }
try { await import('./missing.mjs') } catch (e) { console.log(e.code) }
try { await import('./data.json') } catch (e) { console.log(e.constructor.name, e.code) }
console.log(JSON.stringify(await import('./data.json', { with: { type: 'json' } })));
"#;
    let out = stdout_of(
        &[
            ("main.mjs", main),
            (
                "node_modules/pkg/package.json",
                r#"{ "name": "pkg", "exports": { ".": { "import": "./lib/index.mjs", "require": "./lib/index.cjs" }, "./feature/*": "./lib/feat-*.mjs" } }"#,
            ),
            (
                "node_modules/pkg/lib/index.mjs",
                "export const kind = 'esm'; export default function pkg() { return 'pkg' }\n",
            ),
            ("node_modules/pkg/lib/index.cjs", "exports.kind = 'cjs';\n"),
            (
                "node_modules/pkg/lib/feat-x.mjs",
                "export const feat = 'x';\n",
            ),
            (
                "node_modules/legacy/package.json",
                r#"{ "name": "legacy", "main": "main" }"#,
            ),
            (
                "node_modules/legacy/main.js",
                "module.exports = { a: 1, b: 2 }; module.exports.c = 3;\n",
            ),
            ("data.json", r#"{"x": [1, 2]}"#),
        ],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
pkg esm x { a: 1, b: 2, c: 3 } 3 { x: [ 1, 2 ] }
true node:fs
ERR_PACKAGE_PATH_NOT_EXPORTED
ERR_MODULE_NOT_FOUND true
ERR_MODULE_NOT_FOUND
TypeError ERR_IMPORT_ATTRIBUTE_MISSING
{"default":{"x":[1,2]}}
"#,
    );
}

/// Import bindings are lexical: a parameter, block, catch or destructuring
/// binding of the same name shadows one, and every other reference — in a
/// template, a shorthand property, a class body — reads the live import. A
/// module whose evaluation threw rejects every later `import()` with the same
/// error without running again.
#[test]
fn import_scoping_star_exports_and_cached_errors() {
    let main = r#"import { x as cx } from './one.mjs';
import { star, starLive, bumpStar } from './starer.mjs';
import { ns, nsKeys } from './nsre.mjs';
import fromCjs, { viaStar } from './cjsstar.mjs';
console.log(star, starLive); bumpStar(); console.log(starLive);
console.log(ns.a, nsKeys, fromCjs, viaStar);
for (let i = 0; i < 2; i++) { try { await import('./thrower.mjs') } catch (e) { console.log('caught', e.message) } }
{ const cx = 'block'; console.log(cx) }
try { throw 1 } catch (cx) { console.log(cx) }
function param(cx) { return cx * 10 }
console.log(param(5), [1].map(cx => cx + 1)[0], `${cx}!`, { cx }, typeof cx);
const { a = cx } = {}; console.log(a);
class K { m() { return cx } static s = cx; } console.log(new K().m(), K.s);
"#;
    let star1 = r#"export let starLive = 1;
export function bumpStar() { starLive++ }
export const star = 's';
export default 'nodefault';
"#;
    let nsre = r#"import * as ns from './nsa.mjs';
export { ns };
export const nsKeys = Object.keys(ns);
"#;
    let out = stdout_of(
        &[
            ("main.mjs", main),
            ("one.mjs", "export const x = 'x1';\n"),
            ("star1.mjs", star1),
            ("starer.mjs", "export * from './star1.mjs';\n"),
            ("nsre.mjs", nsre),
            ("nsa.mjs", "export const a = 'A'; export const b = 'B';\n"),
            (
                "cjsstar.mjs",
                "export * from './c.cjs';\nimport d from './c.cjs';\nexport default d;\n",
            ),
            ("c.cjs", "exports.viaStar = 'vs'; exports.other = 1;\n"),
            (
                "thrower.mjs",
                "console.log('thrower runs'); throw new Error('boom');\n",
            ),
        ],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
s 1
2
A [ 'a', 'b' ] { viaStar: 'vs', other: 1 } vs
thrower runs
caught boom
caught boom
block
1
50 2 x1! { cx: 'x1' } string
x1
x1 x1
"#,
    );
}

/// The module early errors (16.2.1.1) and the link-time `SyntaxError`s, in
/// V8's words, observed through `import()` from a CommonJS script.
#[test]
fn module_early_errors_and_link_errors() {
    let cases = [
        "export { nope };",
        "import a from './ok.mjs'; let a;",
        "export let a = 1; export { a };",
        "return 1;",
        "with (Math) {}",
        "import { default } from './ok.mjs';",
        "{ export const z = 1 }",
        "export default async function () {} ; export const t = 1;",
        "let x = 010;",
        "export { x as default, y as default }; var x, y;",
        "import * as ns from './ok.mjs'; ns = 1;",
        "export * as \"str ing\" from './ok.mjs';",
        "export default class { static m() { return 1 } }",
        "import { \"string name\" as sn } from './ok.mjs';",
        "var await = 1;",
        "delete x; var x;",
        "export var v1 = 1, { v2, v3: [v4] } = { v2: 2, v3: [4] };",
        "import { \"a b\" } from './ok.mjs'",
        "export default 1; export default 2;",
        "import { nope } from './ok.mjs';",
        "export * from './s1.mjs'; export * from './s2.mjs'; import { dup } from './amb.mjs';",
    ];
    let mut files: Vec<(String, String)> = cases
        .iter()
        .enumerate()
        .map(|(i, src)| (format!("c{}.mjs", i + 1), format!("{src}\n")))
        .collect();
    files.push((
        "ok.mjs".into(),
        "export default 1; export const q = 1;\n".into(),
    ));
    files.push(("s1.mjs".into(), "export const dup = 1;\n".into()));
    files.push(("s2.mjs".into(), "export const dup = 2;\n".into()));
    files.push((
        "amb.mjs".into(),
        "export * from './s1.mjs'; export * from './s2.mjs';\n".into(),
    ));
    let runner = format!(
        "(async () => {{\n  for (let i = 1; i <= {}; i++) {{\n    try {{ const ns = await import(`./c${{i}}.mjs`); console.log(i, 'ok', Object.keys(ns)) }}\n    catch (e) {{ console.log(i, e.constructor.name, e.message) }}\n  }}\n}})();\n",
        cases.len()
    );
    files.push(("run.cjs".into(), runner));
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let out = stdout_of(&borrowed, &["run.cjs"]);
    assert_lines(
        &out,
        r#"
1 SyntaxError Export 'nope' is not defined in module
2 SyntaxError Identifier 'a' has already been declared
3 SyntaxError Duplicate export of 'a'
4 SyntaxError Illegal return statement
5 SyntaxError Strict mode code may not include a with statement
6 SyntaxError Unexpected reserved word
7 SyntaxError Unexpected token 'export'
8 ok [ 'default', 't' ]
9 SyntaxError Octal literals are not allowed in strict mode.
10 SyntaxError Duplicate export of 'default'
11 TypeError Assignment to constant variable.
12 ok [ 'str ing' ]
13 ok [ 'default' ]
14 SyntaxError The requested module './ok.mjs' does not provide an export named 'string name'
15 SyntaxError Unexpected reserved word
16 SyntaxError Delete of an unqualified identifier in strict mode.
17 ok [ 'v1', 'v2', 'v4' ]
18 SyntaxError Unexpected reserved word
19 SyntaxError Identifier '.default' has already been declared
20 SyntaxError The requested module './ok.mjs' does not provide an export named 'nope'
21 SyntaxError The requested module './amb.mjs' contains conflicting star exports for name 'dup'
"#,
    );
}

/// `import()` from a CommonJS module resolves against THAT module's file, not
/// the entry point's.
#[test]
fn dynamic_import_from_commonjs_resolves_against_the_caller() {
    let out = stdout_of(
        &[
            ("sub/x.js", "module.exports = () => import('./y.mjs');\n"),
            ("sub/y.mjs", "export const y = 'sub y';\n"),
            (
                "main.js",
                "require('./sub/x.js')().then(n => console.log(n.y)); import('./sub/y.mjs').then(n => console.log('direct', n.y));\n",
            ),
        ],
        &["main.js"],
    );
    assert_lines(&out, "sub y\ndirect sub y");
}

/// `#name` specifiers through the package's `"imports"` (a `./` target, a
/// bare-package target chosen by condition KEY ORDER, a `*` pattern), a
/// package importing itself by name through its own `"exports"`, and the
/// error for an import the package does not define.
#[test]
fn package_imports_and_self_reference() {
    let package = r##"{ "name": "selfpkg", "type": "module",
  "exports": { ".": "./src/main.js", "./util": "./src/util.js" },
  "imports": { "#internal/*": "./src/internal/*.js", "#dep": { "node": "dep", "default": "./nope.js" } } }"##;
    let main = r##"import { internal } from '#internal/a';
import { dep } from '#dep';
import { util } from 'selfpkg/util';
console.log(internal, dep, util);
try { await import('#nope') } catch (e) { console.log(e.constructor.name, e.code) }
"##;
    let out = stdout_of(
        &[
            ("package.json", package),
            ("src/main.js", main),
            ("src/internal/a.js", "export const internal = 'int';\n"),
            ("src/util.js", "export const util = 'u';\n"),
            (
                "node_modules/dep/package.json",
                r#"{ "name": "dep", "type": "module", "main": "index.js" }"#,
            ),
            ("node_modules/dep/index.js", "export const dep = 'dep';\n"),
        ],
        &["src/main.js"],
    );
    assert_lines(&out, "int dep u\nTypeError ERR_PACKAGE_IMPORT_NOT_DEFINED");
}

/// An import binding is immutable (9.1.1.5.5): every form of write throws
/// `TypeError: Assignment to constant variable.` — a destructuring target
/// after the elements before it were assigned, a `for` head, a compound
/// assignment after reading the binding — while `??=` on a non-nullish value
/// writes nothing and so throws nothing. No global is ever created.
#[test]
fn writes_to_import_bindings_throw() {
    let main = r#"import { v } from './v.mjs';
let a;
try { for (v of [1]) ; } catch (e) { console.log(e.constructor.name, e.message) }
try { for (v in { k: 1 }) ; } catch (e) { console.log(e.constructor.name, e.message) }
try { [a, v] = [1, 2] } catch (e) { console.log(e.constructor.name, e.message, a) }
try { ({ k: v } = { k: 1 }) } catch (e) { console.log(e.constructor.name, e.message) }
try { v ??= 1; console.log('no write', v) } catch (e) { console.log(e.message) }
try { v += 'x' } catch (e) { console.log(e.constructor.name, e.message, v) }
console.log(typeof globalThis.v);
"#;
    let out = stdout_of(
        &[("main.mjs", main), ("v.mjs", "export let v = 'V';\n")],
        &["main.mjs"],
    );
    assert_lines(
        &out,
        r#"
TypeError Assignment to constant variable.
TypeError Assignment to constant variable.
TypeError Assignment to constant variable. 1
TypeError Assignment to constant variable.
no write V
TypeError Assignment to constant variable. V
undefined
"#,
    );
}

/// `util.inspect` of a namespace whose module has not run yet shows each
/// binding still in its dead zone as `<uninitialized>`.
#[test]
fn inspecting_a_namespace_inside_a_cycle() {
    let out = stdout_of(
        &[
            (
                "u1.mjs",
                "import * as ns from './u2.mjs'; export const early = 1; console.log(ns);\n",
            ),
            (
                "u2.mjs",
                "import * as ns1 from './u1.mjs'; console.log(ns1); export const late = 2;\n",
            ),
        ],
        &["u1.mjs"],
    );
    assert_lines(
        &out,
        "[Module: null prototype] { early: <uninitialized> }\n[Module: null prototype] { late: 2 }",
    );
}
