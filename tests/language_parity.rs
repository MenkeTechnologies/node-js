//! Differential parity tests for the lexer, the early-error pass and the builtin
//! argument checks. Each expected value was captured from system `node`
//! (v26.11.1); the tests drive the built `node` binary as a subprocess, so no
//! Node install is needed in CI.

use std::io::Write;
use std::process::Command;

/// Run `src` through the built `node` binary, returning trimmed stdout. Panics
/// with stderr on a non-zero exit so a thrown error surfaces in the failure.
fn run(src: &str) -> String {
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(src.as_bytes()).expect("write source");
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(f.path())
        .env("TZ", "UTC")
        .output()
        .expect("spawn node binary");
    if !out.status.success() {
        panic!(
            "program failed:\n--- stderr ---\n{}\n--- stdout ---\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Early errors, reported through `eval` so the message is observable. The wording is
/// V8's; the cases pair an invalid snippet with the valid sibling that must keep
/// parsing.
#[test]
fn syntax_errors_use_v8_wording() {
    let src = r##"
const cases = [
  "var 1", "f(1 2)", "f(", "a b", "let x = ;", "1 +", "x = {a b}", "if (1) else 2",
  "x = 'a' 'b'", "var enum", "class { }", "function () {}", "export default 1",
  "import x from 'y'", "a: a: 1", "a: { a: 1 }", "break", "continue", "a: { continue a }",
  "a: { break b }", "return 1", "(function () { return 1 })()", "do ; while (0) 1",
  "throw\n1", "function* g() { yield\n* 2 }", "1 = 2", "a + b = 1", "x?.y = 1", "++1",
  "({ a: 1 }) = 1", "(a) = 1", "[a, ...b, c] = []", "({ ...a, b } = {})",
  "function f() {} f() = 1", "const x", "const [a]", "let let = 1",
  "for (let i, j of []) ;", "for (const x = 1 of []) ;", "for (let i of [1]) { var i }",
  "for (var i of [1]) { var i }", "if (1) const x = 1", "while (0) function f() {}",
  "class A { constructor() {} constructor() {} }", "class A { get a(x) {} }",
  "class A { set a() {} }", "class A { #a; #a }", "class A { get #a() {} set #a(v) {} }",
  "({ __proto__: 1, __proto__: 2 })", "({ a = 1 })", "({ a = 1 } = {})",
  "for ({ a = 1 } of [{}]) ;", "({ a = 1 }) => a",
  "function f(a, a) {}", "function f(a, a) { 'use strict' }", "(a, a) => 1",
  "function f(a = 1) { 'use strict' }", "'use strict'; var static", "'use strict'; 010",
  "'use strict'; '\\101'", "010", "'\\101'", "super.x", "await 1",
  "a ?? b || c", "a || b ?? c", "(a || b) ?? c", "-1 ** 2", "`${}`", "/(/", "/a/gg",
  "if (0) { /(/ }", "a?.`x`", "new a?.b()",
];
for (const src of cases) {
  let r;
  try { r = "ok:" + typeof eval(src); } catch (e) { r = e.name + ": " + e.message; }
  console.log(JSON.stringify(src), "=>", r);
}
"##;
    let expected = r##"
"var 1" => SyntaxError: Unexpected number
"f(1 2)" => SyntaxError: missing ) after argument list
"f(" => SyntaxError: Unexpected end of input
"a b" => SyntaxError: Unexpected identifier 'b'
"let x = ;" => SyntaxError: Unexpected token ';'
"1 +" => SyntaxError: Unexpected end of input
"x = {a b}" => SyntaxError: Unexpected identifier 'b'
"if (1) else 2" => SyntaxError: Unexpected token 'else'
"x = 'a' 'b'" => SyntaxError: Unexpected string
"var enum" => SyntaxError: Unexpected reserved word
"class { }" => SyntaxError: Unexpected token '{'
"function () {}" => SyntaxError: Function statements require a function name
"export default 1" => SyntaxError: Unexpected token 'export'
"import x from 'y'" => SyntaxError: Cannot use import statement outside a module
"a: a: 1" => SyntaxError: Label 'a' has already been declared
"a: { a: 1 }" => SyntaxError: Label 'a' has already been declared
"break" => SyntaxError: Illegal break statement
"continue" => SyntaxError: Illegal continue statement: no surrounding iteration statement
"a: { continue a }" => SyntaxError: Illegal continue statement: 'a' does not denote an iteration statement
"a: { break b }" => SyntaxError: Undefined label 'b'
"return 1" => SyntaxError: Illegal return statement
"(function () { return 1 })()" => ok:number
"do ; while (0) 1" => ok:number
"throw\n1" => SyntaxError: Illegal newline after throw
"function* g() { yield\n* 2 }" => SyntaxError: Unexpected token '*'
"1 = 2" => SyntaxError: Invalid left-hand side in assignment
"a + b = 1" => SyntaxError: Invalid left-hand side in assignment
"x?.y = 1" => SyntaxError: Invalid left-hand side in assignment
"++1" => SyntaxError: Invalid left-hand side expression in prefix operation
"({ a: 1 }) = 1" => SyntaxError: Invalid left-hand side in assignment
"(a) = 1" => ok:number
"[a, ...b, c] = []" => SyntaxError: Rest element must be last element
"({ ...a, b } = {})" => SyntaxError: Rest element must be last element
"function f() {} f() = 1" => ReferenceError: Invalid left-hand side in assignment
"const x" => SyntaxError: Missing initializer in const declaration
"const [a]" => SyntaxError: Missing initializer in destructuring declaration
"let let = 1" => SyntaxError: let is disallowed as a lexically bound name
"for (let i, j of []) ;" => SyntaxError: Invalid left-hand side in for-of loop: Must have a single binding.
"for (const x = 1 of []) ;" => SyntaxError: for-of loop variable declaration may not have an initializer.
"for (let i of [1]) { var i }" => SyntaxError: Identifier 'i' has already been declared
"for (var i of [1]) { var i }" => ok:undefined
"if (1) const x = 1" => SyntaxError: Unexpected token 'const'
"while (0) function f() {}" => SyntaxError: In non-strict mode code, functions can only be declared at top level, inside a block, or as the body of an if statement.
"class A { constructor() {} constructor() {} }" => SyntaxError: A class may only have one constructor
"class A { get a(x) {} }" => SyntaxError: Getter must not have any formal parameters.
"class A { set a() {} }" => SyntaxError: Setter must have exactly one formal parameter.
"class A { #a; #a }" => SyntaxError: Identifier '#a' has already been declared
"class A { get #a() {} set #a(v) {} }" => ok:undefined
"({ __proto__: 1, __proto__: 2 })" => SyntaxError: Duplicate __proto__ fields are not allowed in object literals
"({ a = 1 })" => SyntaxError: Invalid shorthand property initializer
"({ a = 1 } = {})" => ok:object
"for ({ a = 1 } of [{}]) ;" => ok:undefined
"({ a = 1 }) => a" => ok:function
"function f(a, a) {}" => ok:undefined
"function f(a, a) { 'use strict' }" => SyntaxError: Duplicate parameter name not allowed in this context
"(a, a) => 1" => SyntaxError: Duplicate parameter name not allowed in this context
"function f(a = 1) { 'use strict' }" => SyntaxError: Illegal 'use strict' directive in function with non-simple parameter list
"'use strict'; var static" => SyntaxError: Unexpected strict mode reserved word
"'use strict'; 010" => SyntaxError: Octal literals are not allowed in strict mode.
"'use strict'; '\\101'" => SyntaxError: Octal escape sequences are not allowed in strict mode.
"010" => ok:number
"'\\101'" => ok:string
"super.x" => SyntaxError: 'super' keyword unexpected here
"await 1" => SyntaxError: await is only valid in async functions and the top level bodies of modules
"a ?? b || c" => SyntaxError: Unexpected token '||'
"a || b ?? c" => SyntaxError: Unexpected token '??'
"(a || b) ?? c" => ok:number
"-1 ** 2" => SyntaxError: Unary operator used immediately before exponentiation expression. Parenthesis must be used to disambiguate operator precedence
"`${}`" => SyntaxError: Unexpected token '}'
"/(/" => SyntaxError: Invalid regular expression: /(/: Unterminated group
"/a/gg" => SyntaxError: Invalid regular expression flags
"if (0) { /(/ }" => SyntaxError: Invalid regular expression: /(/: Unterminated group
"a?.`x`" => SyntaxError: Invalid tagged template on optional chain
"new a?.b()" => SyntaxError: Invalid optional chain from new expression
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `5..toString()` is `5.` then a member access; separators sit between digits; a
/// literal may not run into an identifier; the legacy octal forms.
#[test]
fn numeric_literals_lex_one_dot_and_separators_between_digits() {
    let src = r##"
const lits = [
  "5..toString()", "5.0.toFixed(1)", "1.e3", ".5e1", "0x10.toString()", "1e3.toString()",
  "1..a", "1.toString()", "1.5.5", "1_000", "0.0_1", "0xAB_CD", "1_0n", "1_", "1__0",
  "0_1", "0_", "0x_1", "1_.5", "1._5", "0x", "0b12", "0o8", "1e", "3in []", "1a",
  "0n", "0x1fn", "1.5n", "01n", "08n", "010", "0777", "08", "09.5", "09e1", "07.5",
  "07.toString()", "07e1", "08_1", "0xFFFFFFFFFFFFFFFFF", "1e400",
];
for (const lit of lits) {
  let r;
  try { const v = eval(lit); r = typeof v === "bigint" ? v + "n" : String(v); } catch (e) { r = e.name + ": " + e.message; }
  console.log(JSON.stringify(lit), r);
}
"##;
    let expected = r##"
"5..toString()" 5
"5.0.toFixed(1)" 5.0
"1.e3" 1000
".5e1" 5
"0x10.toString()" 16
"1e3.toString()" 1000
"1..a" undefined
"1.toString()" SyntaxError: Invalid or unexpected token
"1.5.5" SyntaxError: Unexpected number
"1_000" 1000
"0.0_1" 0.01
"0xAB_CD" 43981
"1_0n" 10n
"1_" SyntaxError: Numeric separators are not allowed at the end of numeric literals
"1__0" SyntaxError: Only one underscore is allowed as numeric separator
"0_1" SyntaxError: Numeric separator can not be used after leading 0.
"0_" SyntaxError: Numeric separator can not be used after leading 0.
"0x_1" SyntaxError: Invalid or unexpected token
"1_.5" SyntaxError: Numeric separators are not allowed at the end of numeric literals
"1._5" SyntaxError: Invalid or unexpected token
"0x" SyntaxError: Invalid or unexpected token
"0b12" SyntaxError: Invalid or unexpected token
"0o8" SyntaxError: Invalid or unexpected token
"1e" SyntaxError: Invalid or unexpected token
"3in []" SyntaxError: Invalid or unexpected token
"1a" SyntaxError: Invalid or unexpected token
"0n" 0n
"0x1fn" 31n
"1.5n" SyntaxError: Invalid or unexpected token
"01n" SyntaxError: Invalid or unexpected token
"08n" SyntaxError: Invalid or unexpected token
"010" 8
"0777" 511
"08" 8
"09.5" 9.5
"09e1" 90
"07.5" SyntaxError: Unexpected number
"07.toString()" 7
"07e1" SyntaxError: Invalid or unexpected token
"08_1" SyntaxError: Invalid or unexpected token
"0xFFFFFFFFFFFFFFFFF" 295147905179352830000
"1e400" Infinity
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// A `${ … }` field ends at its own closing brace even when a regex literal, a
/// comment, a string or a nested template holds a `}` or a quote.
#[test]
fn template_field_extent_is_found_by_lexing() {
    let src = r##"
const input = "it's";
console.log(`'${input.replace(/'/g, `'\\''`)}'`);
console.log(`${"}"}|${/}/.source}|${/{/.source}|${`}`}`);
console.log(`${ /* } ' */ 5 }`);
console.log(`a${`b${`c${1 + 1}`}`}`);
console.log(`${[1, 2].map((x) => `<${x}>`).join("")}`);
console.log(`${(() => { return `}` })()}`);
console.log(`${{ a: 1 }.a}${{ b: { c: 2 } }.b.c}`);
"##;
    let expected = r##"
'it'\''s'
}|}|{|}
5
abc2
<1><2>
}
12
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `\u0061b` spells the identifier `ab`; `"\101"` is a legacy octal escape.
#[test]
fn identifier_unicode_escapes_and_legacy_octal_string_escapes() {
    let src = r##"
var \u0061b = 3;
var a\u0062c = 4;
var \u{61}bcd = 5;
console.log(ab, abc, abcd, ({ \u0061: 1 }).a);
console.log("\101\60\x41\u0041", "\0".length, "\08".length, "\1\12\123\1234".split("").map((c) => c.charCodeAt(0)).join());
console.log("\8\9", "\400".length);
"##;
    let expected = r##"
3 4 5 1
A0AA 1 2 1,10,83,83,52
89 2
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `\w \d \s \b` are ASCII-only (`\s` is WhiteSpace + LineTerminator, so U+FEFF is in
/// and U+0085 is out); `.` stops at \r, U+2028 and U+2029; `^`/`$` under `m` sit
/// beside them too.
#[test]
fn regexp_classes_are_ascii_and_dot_stops_at_every_line_terminator() {
    let src = r##"
const subjects = ["caf\u00e9 \u0663x", "a\u0085b\ufeffc", "a\rb\nc", "a\u2028b\u2029c", "ab cd_ef-gh", "a\r\nb"];
const res = [/\w+/g, /\W+/g, /\d+/g, /\s+/g, /\S+/g, /\b/g, /\B/g, /./g, /./gs, /^./gm, /.$/gm, /^/gm,
  /[\w-]+/g, /[\d-x]/g, /[^\W\d]+/g, /[\b]/, /\u{61}/, /\u{61}/u];
for (const s of subjects) {
  for (const re of res) console.log(JSON.stringify(s), String(re), JSON.stringify(s.match(re)), JSON.stringify(s.replace(re, "[$&]")));
}
"##;
    let expected = r##"
"café ٣x" /\w+/g ["caf","x"] "[caf]é ٣[x]"
"café ٣x" /\W+/g ["é ٣"] "caf[é ٣]x"
"café ٣x" /\d+/g null "café ٣x"
"café ٣x" /\s+/g [" "] "café[ ]٣x"
"café ٣x" /\S+/g ["café","٣x"] "[café] [٣x]"
"café ٣x" /\b/g ["","","",""] "[]caf[]é ٣[]x[]"
"café ٣x" /\B/g ["","","",""] "c[]a[]fé[] []٣x"
"café ٣x" /./g ["c","a","f","é"," ","٣","x"] "[c][a][f][é][ ][٣][x]"
"café ٣x" /./gs ["c","a","f","é"," ","٣","x"] "[c][a][f][é][ ][٣][x]"
"café ٣x" /^./gm ["c"] "[c]afé ٣x"
"café ٣x" /.$/gm ["x"] "café ٣[x]"
"café ٣x" /^/gm [""] "[]café ٣x"
"café ٣x" /[\w-]+/g ["caf","x"] "[caf]é ٣[x]"
"café ٣x" /[\d-x]/g ["x"] "café ٣[x]"
"café ٣x" /[^\W\d]+/g ["caf","x"] "[caf]é ٣[x]"
"café ٣x" /[\b]/ null "café ٣x"
"café ٣x" /\u{61}/ null "café ٣x"
"café ٣x" /\u{61}/u ["a"] "c[a]fé ٣x"
"ab﻿c" /\w+/g ["a","b","c"] "[a][b]﻿[c]"
"ab﻿c" /\W+/g ["","﻿"] "a[]b[﻿]c"
"ab﻿c" /\d+/g null "ab﻿c"
"ab﻿c" /\s+/g ["﻿"] "ab[﻿]c"
"ab﻿c" /\S+/g ["ab","c"] "[ab]﻿[c]"
"ab﻿c" /\b/g ["","","","","",""] "[]a[][]b[]﻿[]c[]"
"ab﻿c" /\B/g null "ab﻿c"
"ab﻿c" /./g ["a","","b","﻿","c"] "[a][][b][﻿][c]"
"ab﻿c" /./gs ["a","","b","﻿","c"] "[a][][b][﻿][c]"
"ab﻿c" /^./gm ["a"] "[a]b﻿c"
"ab﻿c" /.$/gm ["c"] "ab﻿[c]"
"ab﻿c" /^/gm [""] "[]ab﻿c"
"ab﻿c" /[\w-]+/g ["a","b","c"] "[a][b]﻿[c]"
"ab﻿c" /[\d-x]/g null "ab﻿c"
"ab﻿c" /[^\W\d]+/g ["a","b","c"] "[a][b]﻿[c]"
"ab﻿c" /[\b]/ null "ab﻿c"
"ab﻿c" /\u{61}/ null "ab﻿c"
"ab﻿c" /\u{61}/u ["a"] "[a]b﻿c"
"a\rb\nc" /\w+/g ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /\W+/g ["\r","\n"] "a[\r]b[\n]c"
"a\rb\nc" /\d+/g null "a\rb\nc"
"a\rb\nc" /\s+/g ["\r","\n"] "a[\r]b[\n]c"
"a\rb\nc" /\S+/g ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /\b/g ["","","","","",""] "[]a[]\r[]b[]\n[]c[]"
"a\rb\nc" /\B/g null "a\rb\nc"
"a\rb\nc" /./g ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /./gs ["a","\r","b","\n","c"] "[a][\r][b][\n][c]"
"a\rb\nc" /^./gm ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /.$/gm ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /^/gm ["","",""] "[]a\r[]b\n[]c"
"a\rb\nc" /[\w-]+/g ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /[\d-x]/g null "a\rb\nc"
"a\rb\nc" /[^\W\d]+/g ["a","b","c"] "[a]\r[b]\n[c]"
"a\rb\nc" /[\b]/ null "a\rb\nc"
"a\rb\nc" /\u{61}/ null "a\rb\nc"
"a\rb\nc" /\u{61}/u ["a"] "[a]\rb\nc"
"a b c" /\w+/g ["a","b","c"] "[a] [b] [c]"
"a b c" /\W+/g [" "," "] "a[ ]b[ ]c"
"a b c" /\d+/g null "a b c"
"a b c" /\s+/g [" "," "] "a[ ]b[ ]c"
"a b c" /\S+/g ["a","b","c"] "[a] [b] [c]"
"a b c" /\b/g ["","","","","",""] "[]a[] []b[] []c[]"
"a b c" /\B/g null "a b c"
"a b c" /./g ["a","b","c"] "[a] [b] [c]"
"a b c" /./gs ["a"," ","b"," ","c"] "[a][ ][b][ ][c]"
"a b c" /^./gm ["a","b","c"] "[a] [b] [c]"
"a b c" /.$/gm ["a","b","c"] "[a] [b] [c]"
"a b c" /^/gm ["","",""] "[]a []b []c"
"a b c" /[\w-]+/g ["a","b","c"] "[a] [b] [c]"
"a b c" /[\d-x]/g null "a b c"
"a b c" /[^\W\d]+/g ["a","b","c"] "[a] [b] [c]"
"a b c" /[\b]/ null "a b c"
"a b c" /\u{61}/ null "a b c"
"a b c" /\u{61}/u ["a"] "[a] b c"
"ab cd_ef-gh" /\w+/g ["ab","cd_ef","gh"] "[ab] [cd_ef]-[gh]"
"ab cd_ef-gh" /\W+/g [" ","-"] "ab[ ]cd_ef[-]gh"
"ab cd_ef-gh" /\d+/g null "ab cd_ef-gh"
"ab cd_ef-gh" /\s+/g [" "] "ab[ ]cd_ef-gh"
"ab cd_ef-gh" /\S+/g ["ab","cd_ef-gh"] "[ab] [cd_ef-gh]"
"ab cd_ef-gh" /\b/g ["","","","","",""] "[]ab[] []cd_ef[]-[]gh[]"
"ab cd_ef-gh" /\B/g ["","","","","",""] "a[]b c[]d[]_[]e[]f-g[]h"
"ab cd_ef-gh" /./g ["a","b"," ","c","d","_","e","f","-","g","h"] "[a][b][ ][c][d][_][e][f][-][g][h]"
"ab cd_ef-gh" /./gs ["a","b"," ","c","d","_","e","f","-","g","h"] "[a][b][ ][c][d][_][e][f][-][g][h]"
"ab cd_ef-gh" /^./gm ["a"] "[a]b cd_ef-gh"
"ab cd_ef-gh" /.$/gm ["h"] "ab cd_ef-g[h]"
"ab cd_ef-gh" /^/gm [""] "[]ab cd_ef-gh"
"ab cd_ef-gh" /[\w-]+/g ["ab","cd_ef-gh"] "[ab] [cd_ef-gh]"
"ab cd_ef-gh" /[\d-x]/g ["-"] "ab cd_ef[-]gh"
"ab cd_ef-gh" /[^\W\d]+/g ["ab","cd_ef","gh"] "[ab] [cd_ef]-[gh]"
"ab cd_ef-gh" /[\b]/ null "ab cd_ef-gh"
"ab cd_ef-gh" /\u{61}/ null "ab cd_ef-gh"
"ab cd_ef-gh" /\u{61}/u ["a"] "[a]b cd_ef-gh"
"a\r\nb" /\w+/g ["a","b"] "[a]\r\n[b]"
"a\r\nb" /\W+/g ["\r\n"] "a[\r\n]b"
"a\r\nb" /\d+/g null "a\r\nb"
"a\r\nb" /\s+/g ["\r\n"] "a[\r\n]b"
"a\r\nb" /\S+/g ["a","b"] "[a]\r\n[b]"
"a\r\nb" /\b/g ["","","",""] "[]a[]\r\n[]b[]"
"a\r\nb" /\B/g [""] "a\r[]\nb"
"a\r\nb" /./g ["a","b"] "[a]\r\n[b]"
"a\r\nb" /./gs ["a","\r","\n","b"] "[a][\r][\n][b]"
"a\r\nb" /^./gm ["a","b"] "[a]\r\n[b]"
"a\r\nb" /.$/gm ["a","b"] "[a]\r\n[b]"
"a\r\nb" /^/gm ["","",""] "[]a\r[]\n[]b"
"a\r\nb" /[\w-]+/g ["a","b"] "[a]\r\n[b]"
"a\r\nb" /[\d-x]/g null "a\r\nb"
"a\r\nb" /[^\W\d]+/g ["a","b"] "[a]\r\n[b]"
"a\r\nb" /[\b]/ null "a\r\nb"
"a\r\nb" /\u{61}/ null "a\r\nb"
"a\r\nb" /\u{61}/u ["a"] "[a]\r\nb"
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// Pattern grammar errors, with and without `u`: V8's reason text, and the Annex B
/// forms (`{` literal, `\x4`, `\u{61}`, a quantified lookahead) that are NOT errors
/// outside unicode mode.
#[test]
fn regexp_pattern_errors_use_v8_wording() {
    let src = r##"
const pats = ["(", ")", "(a", "[", "[b-a]", "[\\d-x]", "a{2,1}", "a{1", "{", "}", "]", "{1}", "*", "a**", "^*",
  "\\b+", "(?=a)+", "(?<=a)+", "\\", "\\c", "\\x4", "\\u12", "\\u{110000}", "\\u{61}", "\\-", "\\_", "\\1", "\\01",
  "\\k<a>", "(?<a>.)\\k<a>", "(?<a>.)(?<a>.)", "(?<a>.)|(?<a>.)", "(?<>a)", "\\p{L}", "\\p{Foo}", "[\\p{Foo}]",
  "(?i)", "(?P<n>a)", "[\\b]", "[\\B]", "a{,5}", "x{2}{3}"];
for (const flags of ["", "u"]) {
  for (const p of pats) {
    let r;
    try { r = String(new RegExp(p, flags)); } catch (e) { r = e.name + ": " + e.message; }
    console.log(JSON.stringify(flags), JSON.stringify(p), r);
  }
}
console.log(/(?=a)+/.test("a"), "ab".replace(/(?=b)?/g, "-"), /(?!a)*/.exec("b")[0] === "", /\u00/.test("u00"), /\p{Foo}/.test("p{Foo}"));
"##;
    let expected = r##"
"" "(" SyntaxError: Invalid regular expression: /(/: Unterminated group
"" ")" SyntaxError: Invalid regular expression: /)/: Unmatched ')'
"" "(a" SyntaxError: Invalid regular expression: /(a/: Unterminated group
"" "[" SyntaxError: Invalid regular expression: /[/: Unterminated character class
"" "[b-a]" SyntaxError: Invalid regular expression: /[b-a]/: Range out of order in character class
"" "[\\d-x]" /[\d-x]/
"" "a{2,1}" SyntaxError: Invalid regular expression: /a{2,1}/: numbers out of order in {} quantifier
"" "a{1" /a{1/
"" "{" /{/
"" "}" /}/
"" "]" /]/
"" "{1}" SyntaxError: Invalid regular expression: /{1}/: Nothing to repeat
"" "*" SyntaxError: Invalid regular expression: /*/: Nothing to repeat
"" "a**" SyntaxError: Invalid regular expression: /a**/: Nothing to repeat
"" "^*" SyntaxError: Invalid regular expression: /^*/: Nothing to repeat
"" "\\b+" SyntaxError: Invalid regular expression: /\b+/: Nothing to repeat
"" "(?=a)+" /(?=a)+/
"" "(?<=a)+" SyntaxError: Invalid regular expression: /(?<=a)+/: Invalid quantifier
"" "\\" SyntaxError: Invalid regular expression: /\/: \ at end of pattern
"" "\\c" /\c/
"" "\\x4" /\x4/
"" "\\u12" /\u12/
"" "\\u{110000}" /\u{110000}/
"" "\\u{61}" /\u{61}/
"" "\\-" /\-/
"" "\\_" /\_/
"" "\\1" /\1/
"" "\\01" /\01/
"" "\\k<a>" /\k<a>/
"" "(?<a>.)\\k<a>" /(?<a>.)\k<a>/
"" "(?<a>.)(?<a>.)" SyntaxError: Invalid regular expression: /(?<a>.)(?<a>.)/: Duplicate capture group name
"" "(?<a>.)|(?<a>.)" /(?<a>.)|(?<a>.)/
"" "(?<>a)" SyntaxError: Invalid regular expression: /(?<>a)/: Invalid capture group name
"" "\\p{L}" /\p{L}/
"" "\\p{Foo}" /\p{Foo}/
"" "[\\p{Foo}]" /[\p{Foo}]/
"" "(?i)" SyntaxError: Invalid regular expression: /(?i)/: Invalid group
"" "(?P<n>a)" SyntaxError: Invalid regular expression: /(?P<n>a)/: Invalid group
"" "[\\b]" /[\b]/
"" "[\\B]" /[\B]/
"" "a{,5}" /a{,5}/
"" "x{2}{3}" SyntaxError: Invalid regular expression: /x{2}{3}/: Nothing to repeat
"u" "(" SyntaxError: Invalid regular expression: /(/u: Unterminated group
"u" ")" SyntaxError: Invalid regular expression: /)/u: Unmatched ')'
"u" "(a" SyntaxError: Invalid regular expression: /(a/u: Unterminated group
"u" "[" SyntaxError: Invalid regular expression: /[/u: Unterminated character class
"u" "[b-a]" SyntaxError: Invalid regular expression: /[b-a]/u: Range out of order in character class
"u" "[\\d-x]" SyntaxError: Invalid regular expression: /[\d-x]/u: Invalid character class
"u" "a{2,1}" SyntaxError: Invalid regular expression: /a{2,1}/u: numbers out of order in {} quantifier
"u" "a{1" SyntaxError: Invalid regular expression: /a{1/u: Incomplete quantifier
"u" "{" SyntaxError: Invalid regular expression: /{/u: Lone quantifier brackets
"u" "}" SyntaxError: Invalid regular expression: /}/u: Lone quantifier brackets
"u" "]" SyntaxError: Invalid regular expression: /]/u: Lone quantifier brackets
"u" "{1}" SyntaxError: Invalid regular expression: /{1}/u: Nothing to repeat
"u" "*" SyntaxError: Invalid regular expression: /*/u: Nothing to repeat
"u" "a**" SyntaxError: Invalid regular expression: /a**/u: Nothing to repeat
"u" "^*" SyntaxError: Invalid regular expression: /^*/u: Nothing to repeat
"u" "\\b+" SyntaxError: Invalid regular expression: /\b+/u: Nothing to repeat
"u" "(?=a)+" SyntaxError: Invalid regular expression: /(?=a)+/u: Invalid quantifier
"u" "(?<=a)+" SyntaxError: Invalid regular expression: /(?<=a)+/u: Invalid quantifier
"u" "\\" SyntaxError: Invalid regular expression: /\/u: \ at end of pattern
"u" "\\c" SyntaxError: Invalid regular expression: /\c/u: Invalid Unicode escape
"u" "\\x4" SyntaxError: Invalid regular expression: /\x4/u: Invalid escape
"u" "\\u12" SyntaxError: Invalid regular expression: /\u12/u: Invalid Unicode escape
"u" "\\u{110000}" SyntaxError: Invalid regular expression: /\u{110000}/u: Invalid Unicode escape
"u" "\\u{61}" /\u{61}/u
"u" "\\-" SyntaxError: Invalid regular expression: /\-/u: Invalid escape
"u" "\\_" SyntaxError: Invalid regular expression: /\_/u: Invalid escape
"u" "\\1" SyntaxError: Invalid regular expression: /\1/u: Invalid escape
"u" "\\01" SyntaxError: Invalid regular expression: /\01/u: Invalid decimal escape
"u" "\\k<a>" SyntaxError: Invalid regular expression: /\k<a>/u: Invalid named capture referenced
"u" "(?<a>.)\\k<a>" /(?<a>.)\k<a>/u
"u" "(?<a>.)(?<a>.)" SyntaxError: Invalid regular expression: /(?<a>.)(?<a>.)/u: Duplicate capture group name
"u" "(?<a>.)|(?<a>.)" /(?<a>.)|(?<a>.)/u
"u" "(?<>a)" SyntaxError: Invalid regular expression: /(?<>a)/u: Invalid capture group name
"u" "\\p{L}" /\p{L}/u
"u" "\\p{Foo}" SyntaxError: Invalid regular expression: /\p{Foo}/u: Invalid property name
"u" "[\\p{Foo}]" SyntaxError: Invalid regular expression: /[\p{Foo}]/u: Invalid property name in character class
"u" "(?i)" SyntaxError: Invalid regular expression: /(?i)/u: Invalid group
"u" "(?P<n>a)" SyntaxError: Invalid regular expression: /(?P<n>a)/u: Invalid group
"u" "[\\b]" /[\b]/u
"u" "[\\B]" SyntaxError: Invalid regular expression: /[\B]/u: Invalid escape
"u" "a{,5}" SyntaxError: Invalid regular expression: /a{,5}/u: Incomplete quantifier
"u" "x{2}{3}" SyntaxError: Invalid regular expression: /x{2}{3}/u: Nothing to repeat
true -a-b- true true true
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// Where a sticky/global search begins, what `lastIndex` becomes, `match` leaving
/// `lastIndex` alone without `g`/`y`, `matchAll` reading the original's `lastIndex`,
/// and `$<name>` expansion.
#[test]
fn sticky_and_global_regexps_through_string_methods() {
    let src = r##"
const re = (src, flags, li) => { const r = new RegExp(src, flags); r.lastIndex = li; return r; };
let r = re("a", "y", 0);
console.log("aaba".replace(r, "X"), r.lastIndex);
r = re("a", "gy", 0);
console.log("aaba".replace(r, "X"), r.lastIndex, "aaba".match(re("a", "gy", 0)), "xaba".match(re("a", "gy", 0)));
r = re("a", "y", 2);
console.log("aaba".replace(r, "X"), r.lastIndex, "aab".search(re("a", "y", 1)), "baa".search(re("a", "y", 0)));
r = re("b", "", 3);
console.log("abc".match(r).index, r.lastIndex);
r = re("a", "g", 3);
console.log([..."aaaaa".matchAll(r)].map((m) => m.index).join(), r.lastIndex);
console.log([..."aabaa".matchAll(re("a", "gy", 0))].map((m) => m.index).join());
console.log(JSON.stringify("abc".replace(/(?<x>b)/, "[$<x>|$<y>|$<x]")), JSON.stringify("abc".replace(/b/, "[$<x>]")));
console.log("aaa".replace(/a*?/g, "-"), "😀".replace(/(?:)/gu, "-"), "baa".split(/a/y).join("|"));
"##;
    let expected = r##"
Xaba 1
XXba 0 [ 'a', 'a' ] null
aaba 0 0 -1
1 3
3,4 3
0,1
"a[b||$<x]c" "a[$<x>]c"
-a-a-a- -😀- b||
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// Reflect's argument checks: a primitive target is a TypeError for every method, a
/// malformed descriptor throws (a refused one is `false`), `apply`/`construct`
/// check the target before the list, and `construct` names the VALUE it rejects.
#[test]
fn reflect_validates_target_descriptor_and_prototype() {
    let src = r##"
const P = (f) => { try { return String(f()); } catch (e) { return e.name + ": " + e.message; } };
console.log([
  () => Reflect.isExtensible(1), () => Reflect.preventExtensions("s"), () => Reflect.getOwnPropertyDescriptor(null, "a"),
  () => Reflect.setPrototypeOf({}, 1), () => Reflect.setPrototypeOf({}, undefined), () => Reflect.setPrototypeOf(1, null),
].map(P).join("\n"));
for (const d of [1, null, { get: 1 }, { get() {}, value: 1 }, { set() {}, writable: true }, {}]) {
  console.log(P(() => Reflect.defineProperty({}, "k", d)), "/", P(() => Object.defineProperty({}, "k", d)));
}
console.log(Reflect.defineProperty(Object.freeze({}), "k", { value: 1 }), JSON.stringify(Object.getOwnPropertyDescriptor(Object.defineProperty({}, "k", {}), "k")));
console.log(P(() => Reflect.apply(1)), P(() => Reflect.apply(undefined, null, [])), P(() => Reflect.apply(() => 7, null, 1)));
console.log(P(() => Reflect.construct(1, [])), P(() => Reflect.construct(() => {}, [])), P(() => Reflect.construct(Math.max, [])),
  P(() => Reflect.construct(function () {}, [], 1)), P(() => typeof Reflect.construct(Date, [], Object)));
"##;
    let expected = r##"
TypeError: Reflect.isExtensible called on non-object
TypeError: Reflect.preventExtensions called on non-object
TypeError: Reflect.getOwnPropertyDescriptor called on non-object
TypeError: Object prototype may only be an Object or null: 1
TypeError: Object prototype may only be an Object or null: undefined
TypeError: Reflect.setPrototypeOf called on non-object
TypeError: Property description must be an object: 1 / TypeError: Property description must be an object: 1
TypeError: Property description must be an object: null / TypeError: Property description must be an object: null
TypeError: Getter must be a function: 1 / TypeError: Getter must be a function: 1
TypeError: Invalid property descriptor. Cannot both specify accessors and a value or writable attribute, #<Object> / TypeError: Invalid property descriptor. Cannot both specify accessors and a value or writable attribute, #<Object>
TypeError: Invalid property descriptor. Cannot both specify accessors and a value or writable attribute, #<Object> / TypeError: Invalid property descriptor. Cannot both specify accessors and a value or writable attribute, #<Object>
true / [object Object]
false {"writable":false,"enumerable":false,"configurable":false}
TypeError: Function.prototype.apply was called on 1, which is a number and not a function TypeError: Function.prototype.apply was called on undefined, which is undefined and not a function TypeError: CreateListFromArrayLike called on non-object
TypeError: 1 is not a constructor TypeError: () => {} is not a constructor TypeError: function max() { [native code] } is not a constructor TypeError: 1 is not a constructor object
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// ToInt8…ToUint32 send a non-finite number to 0 and wrap modulo 2^32; a clamped
/// array rounds ties to even; an array-like source is read by `length`; `sort`
/// puts NaN last and -0 first; an element is not deletable or redefinable as
/// read-only.
#[test]
fn typed_array_coercion_nonfinite_array_like_and_element_attributes() {
    let src = r##"
const P = (f) => { try { const v = f(); return Array.isArray(v) ? v.map((x) => Object.is(x, -0) ? "-0" : x).join() : String(v); } catch (e) { return e.name + ": " + e.message; } };
const vals = [0.5, 1.5, 2.5, 254.5, 255.5, -0.5, 256, -1, -129, 65536, 2 ** 31, 2 ** 32 + 7, 1e20, NaN, Infinity, -Infinity];
for (const k of ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array"]) {
  console.log(k, P(() => Array.from(globalThis[k].from(vals))));
}
console.log(P(() => Array.from(new Int16Array({ length: 3, 0: 7, 1: "9", 2: null }))), P(() => new Float32Array({ length: 2 }).length));
console.log(P(() => Array.from(new Float32Array([3, NaN, -0, 0, -Infinity, 1]).sort())));
console.log(P(() => new Uint8Array([1n])), P(() => new BigInt64Array([1])));
const u = new Uint8Array(4);
console.log([delete u[0], delete u[9], Reflect.deleteProperty(u, 1)].join());
for (const d of [{ value: 7 }, { value: 7, writable: false }, { value: 7, configurable: false }, { get() {} }]) {
  console.log(P(() => Object.defineProperty(u, 0, d) === u), P(() => Reflect.defineProperty(u, 9, d)));
}
"##;
    let expected = r##"
Int8Array 0,1,2,-2,-1,0,0,-1,127,0,0,7,0,0,0,0
Uint8Array 0,1,2,254,255,0,0,255,127,0,0,7,0,0,0,0
Uint8ClampedArray 0,2,2,254,255,0,255,0,0,255,255,255,255,0,255,0
Int16Array 0,1,2,254,255,0,256,-1,-129,0,0,7,0,0,0,0
Uint16Array 0,1,2,254,255,0,256,65535,65407,0,0,7,0,0,0,0
Int32Array 0,1,2,254,255,0,256,-1,-129,65536,-2147483648,7,1661992960,0,0,0
Uint32Array 0,1,2,254,255,0,256,4294967295,4294967167,65536,2147483648,7,1661992960,0,0,0
7,9,0 2
-Infinity,-0,0,1,3,NaN
TypeError: Cannot convert a BigInt value to a number TypeError: Cannot convert 1 to a BigInt
false,true,false
true false
TypeError: Cannot redefine property: 0 false
TypeError: Cannot redefine property: 0 false
TypeError: Cannot redefine property: 0 false
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `asIntN`/`asUintN`: `bits` is ToIndex, the operand is ToBigInt (a Number is a
/// TypeError), a width past the operand is the identity (no huge allocation), and
/// `BigInt(x)` converts through ToPrimitive first.
#[test]
fn bigint_width_wrapping_and_conversion() {
    let src = r##"
const P = (f) => { try { const v = f(); return typeof v === "bigint" ? v + "n" : String(v); } catch (e) { return e.name + ": " + e.message; } };
for (const b of [0, 1, 8, 64, -1, 2 ** 53, 2 ** 40, 1.9, "8", NaN, undefined]) {
  console.log(String(b), [255n, -129n, 2n ** 64n, -(2n ** 63n), 5, "12", true, null, [7], {}].map((v) => P(() => BigInt.asUintN(b, v)) + "/" + P(() => BigInt.asIntN(b, v))).join(" "));
}
console.log([[7], [], Object(3n), { valueOf() { return 5; } }, { valueOf() { return 1.5; } }, new Date(5), Symbol("s"), null, new String("12")].map((v) => P(() => BigInt(v))).join(" | "));
"##;
    let expected = r##"
0 0n/0n 0n/0n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 0n/0n 0n/0n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 0n/0n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
1 1n/-1n 1n/-1n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 0n/0n 1n/-1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 1n/-1n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
8 255n/-1n 127n/127n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 12n/12n 1n/1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 7n/7n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
64 255n/255n 18446744073709551487n/-129n 0n/0n 9223372036854775808n/-9223372036854775808n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 12n/12n 1n/1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 7n/7n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
-1 RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer
9007199254740992 RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer RangeError: Invalid value: not (convertible to) a safe integer/RangeError: Invalid value: not (convertible to) a safe integer
1099511627776 255n/255n RangeError: Maximum BigInt size exceeded/-129n 18446744073709551616n/18446744073709551616n RangeError: Maximum BigInt size exceeded/-9223372036854775808n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 12n/12n 1n/1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 7n/7n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
1.9 1n/-1n 1n/-1n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 0n/0n 1n/-1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 1n/-1n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
8 255n/-1n 127n/127n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 12n/12n 1n/1n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 7n/7n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
NaN 0n/0n 0n/0n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 0n/0n 0n/0n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 0n/0n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
undefined 0n/0n 0n/0n 0n/0n 0n/0n TypeError: Cannot convert 5 to a BigInt/TypeError: Cannot convert 5 to a BigInt 0n/0n 0n/0n TypeError: Cannot convert null to a BigInt/TypeError: Cannot convert null to a BigInt 0n/0n SyntaxError: Cannot convert [object Object] to a BigInt/SyntaxError: Cannot convert [object Object] to a BigInt
7n | 0n | 3n | 5n | RangeError: The number 1.5 cannot be converted to a BigInt because it is not an integer | 5n | TypeError: Cannot convert Symbol(s) to a BigInt | TypeError: Cannot convert null to a BigInt | 12n
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// A setter called with no argument is NaN; `Date.UTC` clips at ±8.64e15 and yields
/// +0 for a truncated -0.5; a negative year pads as `-0001`; `Date()` is a string.
#[test]
fn date_setters_clip_and_year_padding() {
    let src = r##"
const P = (f) => { try { const v = f(); return Object.is(v, -0) ? "-0" : String(v); } catch (e) { return e.name + ": " + e.message; } };
for (const s of ["setUTCMilliseconds", "setUTCSeconds", "setUTCMinutes", "setUTCHours", "setUTCDate", "setUTCMonth", "setUTCFullYear", "setTime"]) {
  const d = new Date(951782400000);
  console.log(s, P(() => d[s]()), P(() => d[s](NaN)), P(() => new Date(951782400000)[s](1)));
}
for (const args of ["275760, 8, 13", "275760, 8, 13, 0, 0, 0, 1", "-271821, 3, 19, 23, 59, 59, 999", "99", "", "-0.5"]) {
  console.log(args, P(() => eval(`Date.UTC(${args})`)));
}
const d = new Date(-62198755200000);
console.log(d.toISOString(), d.toUTCString(), String(d).slice(0, 15), d.toDateString());
console.log(typeof Date(), Date(0) === Date(1e12), Object.is(new Date(-0.5).getTime(), 0), Object.is(new Date(0).setTime(-0.5), 0));
"##;
    let expected = r##"
setUTCMilliseconds NaN NaN 951782400001
setUTCSeconds NaN NaN 951782401000
setUTCMinutes NaN NaN 951782460000
setUTCHours NaN NaN 951786000000
setUTCDate NaN NaN 949363200000
setUTCMonth NaN NaN 951782400000
setUTCFullYear NaN NaN -62130499200000
setTime NaN NaN 1
275760, 8, 13 8640000000000000
275760, 8, 13, 0, 0, 0, 1 NaN
-271821, 3, 19, 23, 59, 59, 999 NaN
99 915148800000
 NaN
-0.5 -2208988800000
-000001-01-01T00:00:00.000Z Fri, 01 Jan -0001 00:00:00 GMT Fri Jan 01 -000 Fri Jan 01 -0001
string true true true
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// A constructor with no [[Call]] throws, worded by kind; the ones that are also
/// functions answer a plain call. Called through `globalThis`, which used to fail
/// for every builtin global.
#[test]
fn constructors_without_call_behaviour_name_the_missing_new() {
    let src = r##"
const names = ["Map", "Set", "WeakMap", "WeakSet", "WeakRef", "Promise", "ArrayBuffer", "DataView", "Uint8Array",
  "Float64Array", "BigInt64Array", "FinalizationRegistry", "TextEncoder", "URL", "URLSearchParams", "AbortController",
  "Iterator", "Symbol", "Error", "AggregateError", "Array", "Object", "Function", "RegExp", "Boolean", "Number", "String", "Date"];
for (const n of names) {
  let r;
  try { r = "ok " + typeof globalThis[n](); } catch (e) { r = e.name + ": " + e.message; }
  console.log(n, r);
}
console.log(globalThis.parseInt("42px"), globalThis["Number"]("7") + 1, globalThis.Math.max(1, 3), globalThis.String(5).length);
for (const bad of [undefined, null, 1, {}]) {
  try { new AggregateError(bad); } catch (e) { console.log(e.name + ": " + e.message); }
}
"##;
    let expected = r##"
Map TypeError: Constructor Map requires 'new'
Set TypeError: Constructor Set requires 'new'
WeakMap TypeError: Constructor WeakMap requires 'new'
WeakSet TypeError: Constructor WeakSet requires 'new'
WeakRef TypeError: Constructor WeakRef requires 'new'
Promise TypeError: Promise constructor cannot be invoked without 'new'
ArrayBuffer TypeError: Constructor ArrayBuffer requires 'new'
DataView TypeError: Constructor DataView requires 'new'
Uint8Array TypeError: Constructor Uint8Array requires 'new'
Float64Array TypeError: Constructor Float64Array requires 'new'
BigInt64Array TypeError: Constructor BigInt64Array requires 'new'
FinalizationRegistry TypeError: Constructor FinalizationRegistry requires 'new'
TextEncoder TypeError: Class constructor TextEncoder cannot be invoked without 'new'
URL TypeError: Class constructor URL cannot be invoked without 'new'
URLSearchParams TypeError: Class constructor URLSearchParams cannot be invoked without 'new'
AbortController TypeError: Class constructor AbortController cannot be invoked without 'new'
Iterator TypeError: Constructor Iterator requires 'new'
Symbol ok symbol
Error ok object
AggregateError TypeError: undefined is not iterable (cannot read property Symbol(Symbol.iterator))
Array ok object
Object ok object
Function ok function
RegExp ok object
Boolean ok boolean
Number ok number
String ok string
Date ok string
42 8 3 1
TypeError: undefined is not iterable (cannot read property Symbol(Symbol.iterator))
TypeError: object null is not iterable (cannot read property Symbol(Symbol.iterator))
TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator))
TypeError: object is not iterable (cannot read property Symbol(Symbol.iterator))
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `name` and `length` of the timer functions, which no ECMAScript table covers.
#[test]
fn timer_functions_report_node_name_and_length() {
    let src = r##"
const timers = require("timers");
const promises = require("timers/promises");
for (const [label, f] of [
  ["setTimeout", setTimeout], ["setInterval", setInterval], ["setImmediate", setImmediate],
  ["clearTimeout", clearTimeout], ["clearImmediate", clearImmediate], ["nextTick", process.nextTick],
  ["timers.setTimeout", timers.setTimeout], ["promises.setTimeout", promises.setTimeout],
  ["promises.setImmediate", promises.setImmediate],
]) console.log(label, f.name, f.length);
"##;
    let expected = r##"
setTimeout setTimeout 2
setInterval setInterval 2
setImmediate setImmediate 1
clearTimeout clearTimeout 1
clearImmediate clearImmediate 1
nextTick nextTick 1
timers.setTimeout setTimeout 2
promises.setTimeout setTimeout 2
promises.setImmediate setImmediate 1
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}

/// `toFixed()` with no (or NaN) argument is `toFixed(0)`.
#[test]
fn to_fixed_without_an_argument_is_zero_digits() {
    let src = r##"
console.log((0.5).toFixed(), (2.5).toFixed(), (1.45).toFixed(undefined), (5).toFixed(), (1.5).toFixed(NaN), 0.5.toFixed(), 5..toFixed());
"##;
    let expected = r##"
1 3 1 5 2 1 5
"##;
    assert_eq!(run(src), expected.trim_matches('\n'));
}
