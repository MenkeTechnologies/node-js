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

// ── round 2: fdlibm Math, with, mapped arguments, capture reset, iterator close … ──

// ── round 2: fdlibm Math, with, mapped arguments, capture reset, iterator close ──

/// Math transcendentals are V8's own fdlibm port (with the reference build's fused multiply-adds), not the platform libm.
#[test]
fn r2_math_fdlibm() {
    let src = r##"
// V8's own fdlibm port, not the platform libm: the last digit differs from the
// OS routines on a few percent of inputs (and between operating systems).
const pts = {
  acos: [0.5, 0.2, 0.3], acosh: [3, 1.4697952512651682, 1.2531569674611092],
  asin: [0.5, -0.5, 0.3], asinh: [1.0000001, 0.6745, 1.0986122886681098],
  atan: [0.5, -0.5, 2], atanh: [0.5, 0.00001, 0.7],
  cbrt: [3.141592653589793, -3.141592653589793, 20],
  cos: [20, 0.1, 1000, 1e10, 7839423874.3931055, 2442368315.5328035],
  cosh: [2.356194490192345, 709.7827, 710],
  exp: [0.99, 1.0986122886681098, -5.97647409270798, 1],
  expm1: [1, 0.9, 1.0986122886681098],
  log: [2.356194490192345, 710.4758600739439, 3],
  log10: [3.141592653589793, 2.589974375417494, 1e-5],
  log1p: [2, 0.2, -0.3014574636015158],
  log2: [1.512998185120523, 1.3936608489602804, 10],
  sin: [3.7495980865629996, -43097.51377654933, -204.9476324098623, 1e10, 7839423874.3931055, 1e22],
  sinh: [2, -2, 3.141592653589793],
  tan: [1, -1, 20, 5938765876.926481, 6631970780.435949, 1.5707963267948966],
  tanh: [0.7, 0.99, 1.0000001],
};
for (const [fn, xs] of Object.entries(pts)) console.log(fn, xs.map((x) => Math[fn](x)).join(' '));
console.log(Math.atan2(1, 2), Math.atan2(-3.5, 0.25), Math.atan2(0.1, -7), Math.atan2(-0, -0), Math.atan2(5, Infinity));
console.log(Math.pow(2, 0.5), Math.hypot(3, 4), Math.sqrt(2), Math.exp(710), Math.exp(-746), Math.sinh(710.4758600739439), Math.cosh(-710.4758600739439));
"##;
    let expected = r##"
acos 1.0471975511965979 1.369438406004566 1.2661036727794992
acosh 1.7627471740390859 0.9349030762674887 0.6973417958547874
asin 0.5235987755982989 -0.5235987755982989 0.3046926540153975
asinh 0.8813736577302195 0.6316510591428517 0.9494131316918537
atan 0.4636476090008061 -0.4636476090008061 1.1071487177940904
atanh 0.5493061443340548 0.000010000000000333335 0.8673005276940531
cbrt 1.4645918875615231 -1.4645918875615231 2.7144176165949063
cos 0.40808206181339196 0.9950041652780257 0.5623790762907029 0.873119622676856 -0.8005125160765673 -0.11330348615670452
cosh 5.322752149519959 8.988349783319008e+307 1.1169973830808557e+308
exp 2.6912344723492625 3 0.002537758436996749 2.718281828459045
expm1 1.718281828459045 1.4596031111569499 2
log 0.8570478133976192 6.565934970990844 1.0986122886681096
log10 0.4971498726941338 0.41329546729753175 -5
log1p 1.0986122886681096 0.18232155679395462 -0.3587592053626123
log2 0.5974102570066053 0.4788795202784467 3.321928094887362
sin -0.5712314787562416 -0.9110215292098485 0.6773172597438981 -0.4875060250875107 -0.5993160364989106 -0.8522008497671888
sinh 3.626860407847019 -3.626860407847019 11.548739357257748
tan 1.5574077246549023 -1.5574077246549023 2.237160944224742 2.0682905512269794 1.3300676088320986 16331239353195370
tanh 0.6043677771171636 0.7573623242165262 0.7615941979531959
0.4636476090008061 -1.4994888620096063 3.1273079110023967 -3.141592653589793 0
1.4142135623730951 5 1.4142135623730951 Infinity 0 1.7976931348621744e+308 1.7976931348621744e+308
"##;
    assert_eq!(run(src), expected.trim());
}

/// Shortest round-trip digits: an exact tie takes the even last digit.
#[test]
fn r2_number_tostring_ties() {
    let src = r##"
// Shortest round-trip digits: an exact tie between two candidates takes the EVEN last digit.
console.log(25577030267034.8125, 6.0487079387530684 ** 17, 8.869114365428686 ** 14.62094928137958);
console.log((25577030267034.8125).toExponential(), (25577030267034.8125).toPrecision(17), String(1124215372052873.25));
console.log(0.1 + 0.2, 1 / 3, 2 ** 70, 5e-324, 1.7976931348623157e308, 123456789.12345678);
"##;
    let expected = r##"
25577030267034.812 19420681127215.562 72261859708629.62
2.5577030267034812e+13 25577030267034.813 1124215372052873.2
0.30000000000000004 0.3333333333333333 1.1805916207174113e+21 5e-324 1.7976931348623157e+308 123456789.12345678
"##;
    assert_eq!(run(src), expected.trim());
}

/// parseInt/Number radix algorithms and the StringNumericLiteral grammar.
#[test]
fn r2_number_parse_radix() {
    let src = r##"
// parseInt past 2^53 in radixes 2/4/8/16/32 rounds half-to-even exactly; the other radixes use V8's
// 32-bit chunked accumulation, which is only approximately the true value.
const s = ['9007199254740993', '123456789012345678901234567890', 'zzzzzzzzzzzzzzzzzzzz', 'ffffffffffffffffffffff',
  '0x1fffffffffffff1', '0x20000000000001', '0x20000000000003', '0b' + '1'.repeat(70), '0o' + '7'.repeat(30)];
for (const x of s) console.log(JSON.stringify(x), Number(x), parseInt(x, 36), parseInt(x, 16), parseInt(x, 8), parseInt(x, 32), parseInt(x, 7), parseInt(x));
// StringNumericLiteral grammar: Rust's float parser accepts more than JS does.
for (const x of ['infinity', 'inf', 'nan', '1e', '.5', '5.', '+.5e1', '-.5', '1_0', '0x', '+0x10', '1e+', '  12\n', 'Infinity', '-Infinity', '+Infinity', '0b102', '0o8'])
  console.log(JSON.stringify(x), Number(x));
console.log(Infinity.toPrecision(-1), NaN.toExponential(200), Infinity.toExponential(-3));
"##;
    let expected = r##"
"9007199254740993" 9007199254740992 1.9896986116031812e+24 10378291982571407000 NaN 3.400185036980776e+23 NaN 9007199254740992
"123456789012345678901234567890" 1.2345678901234568e+29 1.436287679432363e+45 9.452287968736547e+34 342391 4.752541744701159e+43 22875 1.2345678901234568e+29
"zzzzzzzzzzzzzzzzzzzz" NaN 1.3367494538843734e+31 NaN NaN NaN NaN NaN
"ffffffffffffffffffffff" NaN 7.424688395289205e+33 3.094850098213451e+26 NaN 6.2810042643566465e+32 NaN NaN
"0x1fffffffffffff1" 144115188075855860 7.304212125376293e+24 144115188075855860 0 0 0 144115188075855860
"0x20000000000001" 9007199254740992 2.0299225653369807e+23 9007199254740992 0 0 0 9007199254740992
"0x20000000000003" 9007199254740996 2.0299225653369807e+23 9007199254740996 0 0 0 9007199254740996
"0b1111111111111111111111111111111111111111111111111111111111111111111111" 1.1805916207174113e+21 9.63150853960049e+109 2.1498869073964735e+85 0 2.530246860221305e+106 0 0
"0o777777777777777777777777777777" 1.2379400392853803e+27 1.18274300713268e+48 0 0 3.4576226362005674e+46 0 0
"infinity" NaN
"inf" NaN
"nan" NaN
"1e" NaN
".5" 0.5
"5." 5
"+.5e1" 5
"-.5" -0.5
"1_0" NaN
"0x" NaN
"+0x10" NaN
"1e+" NaN
"  12\n" 12
"Infinity" Infinity
"-Infinity" -Infinity
"+Infinity" Infinity
"0b102" NaN
"0o8" NaN
Infinity NaN Infinity
"##;
    assert_eq!(run(src), expected.trim());
}

/// JSON.parse escape errors; JSON.stringify with a property list over exotics, boxed symbols and empty results.
#[test]
fn r2_json_edges() {
    let src = r##"
const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const s of ['"\\x41"', '"\\u00"', '"\\u00zz"', '"\\ud83d\\ude00"', '"a\\', '"\\u004', '"\\q"', '"\\uD83D\\uDE00!"', '"\\u0041\\u00e9"'])
  console.log(JSON.stringify(s), P(() => JSON.stringify(JSON.parse(s))));
console.log(P(() => JSON.stringify(new Set([1]), ['1', 0], 1)));
console.log(P(() => JSON.stringify(/re/g, (k, v) => v, '\t')));
console.log(P(() => JSON.stringify(Object(Symbol('b')))), P(() => JSON.stringify([Object(Symbol('b'))])), P(() => JSON.stringify({ k: Object(Symbol('b')) })));
console.log(P(() => JSON.stringify(new Uint8Array([1, 2]), ['1', 0])));
console.log(P(() => JSON.stringify(new Uint8Array([1, 2]), ['1', 0], 2)));
console.log(P(() => JSON.stringify(Object.create({ a: 1 }), ['a'])), P(() => JSON.stringify(Object.defineProperty({}, 'x', { value: 1 }), ['x'])));
console.log(P(() => JSON.stringify(Object.assign(new Map(), { a: 1 }), null, 1)), P(() => JSON.stringify(Math, null, 2)));
"##;
    let expected = r##"
"\"\\x41\"" SyntaxError: Bad escaped character in JSON at position 2 (line 1 column 3)
"\"\\u00\"" SyntaxError: Bad Unicode escape in JSON at position 5 (line 1 column 6)
"\"\\u00zz\"" SyntaxError: Bad Unicode escape in JSON at position 5 (line 1 column 6)
"\"\\ud83d\\ude00\"" "😀"
"\"a\\" SyntaxError: Unexpected end of JSON input
"\"\\u004" SyntaxError: Bad Unicode escape in JSON at position 6 (line 1 column 7)
"\"\\q\"" SyntaxError: Bad escaped character in JSON at position 2 (line 1 column 3)
"\"\\uD83D\\uDE00!\"" "😀!"
"\"\\u0041\\u00e9\"" "Aé"
{}
{}
{} [{}] {"k":{}}
{"1":2,"0":1}
{
  "1": 2,
  "0": 1
}
{"a":1} {"x":1}
{
 "a": 1
} {}
"##;
    assert_eq!(run(src), expected.trim());
}

/// Array mutators on array-likes keep holes and length rules; ArraySpeciesCreate with non-array constructors.
#[test]
fn r2_array_generic_species() {
    let src = r##"
const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const lk of [{ length: 1.9, 0: 4 }, { length: 3, 0: 1, 2: 3 }, { length: '2', 0: 'a', 1: 'b' }, { length: -1 }, { length: NaN, 0: 1 }, { length: 3, 1: 'x' }]) {
  for (const m of ['push', 'pop', 'shift', 'unshift', 'reverse', 'sort', 'splice', 'fill', 'copyWithin']) {
    const o = Object.assign({}, lk);
    const args = { push: [9], unshift: [0], splice: [0, 1], fill: ['f'], copyWithin: [0, 1] }[m] || [];
    console.log(JSON.stringify(lk), m, P(() => Array.prototype[m].apply(o, args)), JSON.stringify(o), Object.keys(o).join());
  }
}
console.log(P(() => Array.prototype.push.call('abc', 1)));
class MyArr extends Array {}
const mkB = (S) => { class B extends Array { static get [Symbol.species]() { return S; } } return B.from([1, 2, 3]); };
const desc = (d) => Object.prototype.toString.call(d) + ':' + (d instanceof Array) + ':' + d.length + ':' + Object.keys(d).join();
for (const [label, S] of [['Object', Object], ['one', 1], ['fnLen', function (n) { return { length: n }; }], ['fn0', function () { return { length: 0 }; }], ['Array', Array], ['undef', undefined], ['null', null], ['arrow', () => {}]]) {
  for (const m of ['map((x) => x)', 'filter(() => true)', 'slice()', 'splice(0, 1)', 'concat([1])', 'flat()', 'flatMap((x) => [x])']) {
    console.log(label, m, P(() => desc(eval('mkB(S).' + m))));
  }
}
"##;
    let expected = r##"
{"0":4,"length":1.9} push 2 {"0":4,"1":9,"length":2} 0,1,length
{"0":4,"length":1.9} pop 4 {"length":0} length
{"0":4,"length":1.9} shift 4 {"length":0} length
{"0":4,"length":1.9} unshift 2 {"0":0,"1":4,"length":2} 0,1,length
{"0":4,"length":1.9} reverse [object Object] {"0":4,"length":1.9} 0,length
{"0":4,"length":1.9} sort [object Object] {"0":4,"length":1.9} 0,length
{"0":4,"length":1.9} splice 4 {"length":0} length
{"0":4,"length":1.9} fill [object Object] {"0":"f","length":1.9} 0,length
{"0":4,"length":1.9} copyWithin [object Object] {"0":4,"length":1.9} 0,length
{"0":1,"2":3,"length":3} push 4 {"0":1,"2":3,"3":9,"length":4} 0,2,3,length
{"0":1,"2":3,"length":3} pop 3 {"0":1,"length":2} 0,length
{"0":1,"2":3,"length":3} shift 1 {"1":3,"length":2} 1,length
{"0":1,"2":3,"length":3} unshift 4 {"0":0,"1":1,"3":3,"length":4} 0,1,3,length
{"0":1,"2":3,"length":3} reverse [object Object] {"0":3,"2":1,"length":3} 0,2,length
{"0":1,"2":3,"length":3} sort [object Object] {"0":1,"1":3,"length":3} 0,1,length
{"0":1,"2":3,"length":3} splice 1 {"1":3,"length":2} 1,length
{"0":1,"2":3,"length":3} fill [object Object] {"0":"f","1":"f","2":"f","length":3} 0,1,2,length
{"0":1,"2":3,"length":3} copyWithin [object Object] {"1":3,"2":3,"length":3} 1,2,length
{"0":"a","1":"b","length":"2"} push 3 {"0":"a","1":"b","2":9,"length":3} 0,1,2,length
{"0":"a","1":"b","length":"2"} pop b {"0":"a","length":1} 0,length
{"0":"a","1":"b","length":"2"} shift a {"0":"b","length":1} 0,length
{"0":"a","1":"b","length":"2"} unshift 3 {"0":0,"1":"a","2":"b","length":3} 0,1,2,length
{"0":"a","1":"b","length":"2"} reverse [object Object] {"0":"b","1":"a","length":"2"} 0,1,length
{"0":"a","1":"b","length":"2"} sort [object Object] {"0":"a","1":"b","length":"2"} 0,1,length
{"0":"a","1":"b","length":"2"} splice a {"0":"b","length":1} 0,length
{"0":"a","1":"b","length":"2"} fill [object Object] {"0":"f","1":"f","length":"2"} 0,1,length
{"0":"a","1":"b","length":"2"} copyWithin [object Object] {"0":"b","1":"b","length":"2"} 0,1,length
{"length":-1} push 1 {"0":9,"length":1} 0,length
{"length":-1} pop undefined {"length":0} length
{"length":-1} shift undefined {"length":0} length
{"length":-1} unshift 1 {"0":0,"length":1} 0,length
{"length":-1} reverse [object Object] {"length":-1} length
{"length":-1} sort [object Object] {"length":-1} length
{"length":-1} splice  {"length":0} length
{"length":-1} fill [object Object] {"length":-1} length
{"length":-1} copyWithin [object Object] {"length":-1} length
{"0":1,"length":null} push 1 {"0":9,"length":1} 0,length
{"0":1,"length":null} pop undefined {"0":1,"length":0} 0,length
{"0":1,"length":null} shift undefined {"0":1,"length":0} 0,length
{"0":1,"length":null} unshift 1 {"0":0,"length":1} 0,length
{"0":1,"length":null} reverse [object Object] {"0":1,"length":null} 0,length
{"0":1,"length":null} sort [object Object] {"0":1,"length":null} 0,length
{"0":1,"length":null} splice  {"0":1,"length":0} 0,length
{"0":1,"length":null} fill [object Object] {"0":1,"length":null} 0,length
{"0":1,"length":null} copyWithin [object Object] {"0":1,"length":null} 0,length
{"1":"x","length":3} push 4 {"1":"x","3":9,"length":4} 1,3,length
{"1":"x","length":3} pop undefined {"1":"x","length":2} 1,length
{"1":"x","length":3} shift undefined {"0":"x","length":2} 0,length
{"1":"x","length":3} unshift 4 {"0":0,"2":"x","length":4} 0,2,length
{"1":"x","length":3} reverse [object Object] {"1":"x","length":3} 1,length
{"1":"x","length":3} sort [object Object] {"0":"x","length":3} 0,length
{"1":"x","length":3} splice  {"0":"x","length":2} 0,length
{"1":"x","length":3} fill [object Object] {"0":"f","1":"f","2":"f","length":3} 0,1,2,length
{"1":"x","length":3} copyWithin [object Object] {"0":"x","length":3} 0,length
TypeError: Cannot assign to read only property 'length' of object '[object String]'
Object map((x) => x) [object Number]:false:undefined:0,1,2
Object filter(() => true) [object Number]:false:undefined:0,1,2
Object slice() [object Number]:false:3:0,1,2,length
Object splice(0, 1) [object Number]:false:1:0,length
Object concat([1]) [object Number]:false:4:0,1,2,3,length
Object flat() [object Number]:false:undefined:0,1,2
Object flatMap((x) => [x]) [object Number]:false:undefined:0,1,2
one map((x) => x) TypeError: object.constructor[Symbol.species] is not a constructor
one filter(() => true) TypeError: object.constructor[Symbol.species] is not a constructor
one slice() TypeError: object.constructor[Symbol.species] is not a constructor
one splice(0, 1) TypeError: object.constructor[Symbol.species] is not a constructor
one concat([1]) TypeError: object.constructor[Symbol.species] is not a constructor
one flat() TypeError: object.constructor[Symbol.species] is not a constructor
one flatMap((x) => [x]) TypeError: object.constructor[Symbol.species] is not a constructor
fnLen map((x) => x) [object Object]:false:3:0,1,2,length
fnLen filter(() => true) [object Object]:false:0:0,1,2,length
fnLen slice() [object Object]:false:3:0,1,2,length
fnLen splice(0, 1) [object Object]:false:1:0,length
fnLen concat([1]) [object Object]:false:4:0,1,2,3,length
fnLen flat() [object Object]:false:0:0,1,2,length
fnLen flatMap((x) => [x]) [object Object]:false:0:0,1,2,length
fn0 map((x) => x) [object Object]:false:0:0,1,2,length
fn0 filter(() => true) [object Object]:false:0:0,1,2,length
fn0 slice() [object Object]:false:3:0,1,2,length
fn0 splice(0, 1) [object Object]:false:1:0,length
fn0 concat([1]) [object Object]:false:4:0,1,2,3,length
fn0 flat() [object Object]:false:0:0,1,2,length
fn0 flatMap((x) => [x]) [object Object]:false:0:0,1,2,length
Array map((x) => x) [object Array]:true:3:0,1,2
Array filter(() => true) [object Array]:true:3:0,1,2
Array slice() [object Array]:true:3:0,1,2
Array splice(0, 1) [object Array]:true:1:0
Array concat([1]) [object Array]:true:4:0,1,2,3
Array flat() [object Array]:true:3:0,1,2
Array flatMap((x) => [x]) [object Array]:true:3:0,1,2
undef map((x) => x) [object Array]:true:3:0,1,2
undef filter(() => true) [object Array]:true:3:0,1,2
undef slice() [object Array]:true:3:0,1,2
undef splice(0, 1) [object Array]:true:1:0
undef concat([1]) [object Array]:true:4:0,1,2,3
undef flat() [object Array]:true:3:0,1,2
undef flatMap((x) => [x]) [object Array]:true:3:0,1,2
null map((x) => x) [object Array]:true:3:0,1,2
null filter(() => true) [object Array]:true:3:0,1,2
null slice() [object Array]:true:3:0,1,2
null splice(0, 1) [object Array]:true:1:0
null concat([1]) [object Array]:true:4:0,1,2,3
null flat() [object Array]:true:3:0,1,2
null flatMap((x) => [x]) [object Array]:true:3:0,1,2
arrow map((x) => x) TypeError: object.constructor[Symbol.species] is not a constructor
arrow filter(() => true) TypeError: object.constructor[Symbol.species] is not a constructor
arrow slice() TypeError: object.constructor[Symbol.species] is not a constructor
arrow splice(0, 1) TypeError: object.constructor[Symbol.species] is not a constructor
arrow concat([1]) TypeError: object.constructor[Symbol.species] is not a constructor
arrow flat() TypeError: object.constructor[Symbol.species] is not a constructor
arrow flatMap((x) => [x]) TypeError: object.constructor[Symbol.species] is not a constructor
"##;
    assert_eq!(run(src), expected.trim());
}

/// Sloppy-mode mapped arguments object aliases the parameters.
#[test]
fn r2_mapped_arguments() {
    let src = r##"
function f(a, b) { arguments[0] = 7; b = 9; return [a, arguments[1], arguments.length]; }
console.log(f(1, 2), f(1), f());
function g(a) { a = 5; return arguments[0]; }
console.log(g(1), g());
function h(a) { 'use strict'; arguments[0] = 7; a = 3; return [a, arguments[0]]; }
console.log(h(1));
function k(a, a2) { delete arguments[0]; a = 9; return [arguments[0], a]; }
console.log(k(1, 2));
function d(a, a) { arguments[0] = 'x'; arguments[1] = 'y'; return a; }
console.log(d(1, 2));
function c(a) { const cl = () => { a = 11; }; cl(); return arguments[0]; }
console.log(c(1));
function dflt(a, b = 2) { arguments[0] = 9; return a; }
console.log(dflt(1));
function va(a) { var a = 4; return arguments[0]; }
console.log(va(1));
function ev(a) { eval('a = 8'); return arguments[0]; }
console.log(ev(1));
function ar(a) { return (() => { arguments[0] = 6; return a; })(); }
console.log(ar(1));
function sw(a, b) { [].reverse.call(arguments); return [a, b, arguments[0], arguments[1]]; }
console.log(sw(1, 2));
"##;
    let expected = r##"
[ 7, 9, 2 ] [ 7, undefined, 1 ] [ undefined, undefined, 0 ]
5 undefined
[ 3, 7 ]
[ undefined, 9 ]
y
11
1
4
8
6
[ 2, 1, 2, 1 ]
"##;
    assert_eq!(run(src), expected.trim());
}

/// The with statement, Symbol.unscopables and direct eval inside it.
#[test]
fn r2_with_statement() {
    let src = r##"
var o = { a: 1, b: 2, f() { return this === o; } };
var a = 'outer', c = 'c';
with (o) {
  console.log(a, b, c, typeof a, typeof zzz);
  a = 10; c = 'C2';
  var d = 5;
  console.log(f(), (() => a)());
  var a = 77;
}
console.log(o.a, c, d, a);
with ({ x: 1 }) { var x = 2; }
console.log(x);
with ([1, 2, 3]) { console.log(length, typeof push, join('-')); }
with ('abc') { console.log(length, toUpperCase()); }
try { with (null) {} } catch (e) { console.log(e.constructor.name, e.message); }
var p = { v: 1 };
function g() { with (p) { v++; v += 2; return function () { return v; }; } }
var h = g(); p.v = 100; console.log(h(), p.v);
with ({ [Symbol.unscopables]: { hid: true }, hid: 5, vis: 6 }) { var hid = 'x'; console.log(typeof vis, typeof hid, hid); }
console.log(hid);
with (Math) { console.log(max(1, 2), PI > 3, floor(2.5)); }
function k(arg) { with (arg) { return z; } }
try { k({}); } catch (e) { console.log(e.name + ': ' + e.message); }
label: with ({}) { break label; }
for (var i = 0; i < 3; i++) { with ({ i2: i }) { if (i2 == 1) continue; console.log(i2); } }
var q = { a: 1 };
with (q) { console.log(eval('a'), eval('a = 5; a'), q.a); eval('var qq = 9'); }
console.log(qq);
function s() { 'use strict'; try { eval('with ({}) {}'); } catch (e) { console.log(e.name, e.message); } }
s();
try { eval('"use strict"; with ({}) {}'); } catch (e) { console.log(e.name, e.message); }
console.log(new Function('o', 'with (o) { return x + 1 }')({ x: 4 }));
with ({ g: function () { return typeof this; } }) console.log(g());
"##;
    let expected = r##"
1 2 c number undefined
true 10
77 C2 5 outer
undefined
3 function 1-2-3
3 ABC
TypeError Cannot convert undefined or null to object
100 100
number string x
x
2 true 2
ReferenceError: z is not defined
0
2
1 5 5
9
SyntaxError Strict mode code may not include a with statement
SyntaxError Strict mode code may not include a with statement
5
object
"##;
    assert_eq!(run(src), expected.trim());
}

/// super.x = v, compound and update forms, static and object-literal homes.
#[test]
fn r2_super_assignment() {
    let src = r##"
class A { get v() { return this._v; } set v(x) { this._v = x; } static set sv(x) { A._sv = x; } }
class B extends A {
  set v(x) { super.v = x + '!'; }
  get v() { return 'B>' + super.v; }
  inc() { super.v += 5; super['v'] *= 2; return this._v; }
  upd() { super.v++; return this._v; }
  setNew() { super.fresh = 1; return Object.keys(this).join(); }
  static st() { super.sv = 'S'; return A._sv; }
}
const b = new B(); b._v = 1;
console.log(b.inc(), b.upd(), b.setNew(), B.st());
b.v = 'x'; console.log(b._v, b.v);
const o = { __proto__: { set p(x) { this.got = x; } }, m() { super.p = 7; return this.got; } };
console.log(o.m());
class R { get ro() { return 1; } }
class S extends R { w() { 'use strict'; try { super.ro = 2; } catch (e) { return e.constructor.name; } return 'no throw'; } }
console.log(new S().w());
"##;
    let expected = r##"
12 13 _v,fresh S
x! B>x!
7
TypeError
"##;
    assert_eq!(run(src), expected.trim());
}

/// Captures inside a quantified group reset on every iteration; empty iterations are rejected under min 0.
#[test]
fn r2_regexp_capture_reset() {
    let src = r##"
// RepeatMatcher step 4 clears the captures inside a group at the start of every iteration.
console.log(/(z)((a+)?(b+)?(c))*/.exec('zaacbbbcac'));
console.log(/(?:(a)|b)*/.exec('ab'));
console.log(/(?:(a)|(b))+/.exec('ab'));
console.log(/(a)|b/.exec('b'));
console.log(/(?:(a)|b){2}/.exec('ab'), /(?:(a)|b){2}/.exec('ba'));
console.log('aab'.replace(/(?:(a)|(b))+/, '[$1|$2]'));
console.log(/(?<x>a)|(?<y>b)/.exec('b').groups);
console.log(/(?:(?:(a)|b)c)*/.exec('acbc'));
console.log(/(a*)*/.exec('b'), /(a*)+/.exec('b'), /(a*)?/.exec('b'), /(?:(a*))?/.exec('b'));
console.log(/(?:(a)|(b)){0,2}x/.exec('abx'));
console.log('abc'.split(/(?:(a)|(b))+/));
console.log([...'ab'.matchAll(/(?:(a)|(b))+/g)].map((m) => m.slice()));
console.log(/(?:a(b)?)+/.exec('aba'), /(?:(?=(a))a|b)+/.exec('ab'));
console.log(/(a*)*b/.exec('aab'), /(a|)*/.exec('ab'), /(a*){2,}/.exec('aa'));
"##;
    let expected = r##"
[
  'zaacbbbcac',
  'z',
  'ac',
  'a',
  undefined,
  'c',
  index: 0,
  input: 'zaacbbbcac',
  groups: undefined
]
[ 'ab', undefined, index: 0, input: 'ab', groups: undefined ]
[ 'ab', undefined, 'b', index: 0, input: 'ab', groups: undefined ]
[ 'b', undefined, index: 0, input: 'b', groups: undefined ]
[ 'ab', undefined, index: 0, input: 'ab', groups: undefined ] [ 'ba', 'a', index: 0, input: 'ba', groups: undefined ]
[|b]
[Object: null prototype] { x: undefined, y: 'b' }
[ 'acbc', undefined, index: 0, input: 'acbc', groups: undefined ]
[ '', undefined, index: 0, input: 'b', groups: undefined ] [ '', '', index: 0, input: 'b', groups: undefined ] [ '', undefined, index: 0, input: 'b', groups: undefined ] [ '', undefined, index: 0, input: 'b', groups: undefined ]
[ 'abx', undefined, 'b', index: 0, input: 'abx', groups: undefined ]
[ '', undefined, 'b', 'c' ]
[ [ 'ab', undefined, 'b' ] ]
[ 'aba', undefined, index: 0, input: 'aba', groups: undefined ] [ 'ab', undefined, index: 0, input: 'ab', groups: undefined ]
[ 'aab', 'aa', index: 0, input: 'aab', groups: undefined ] [ 'a', 'a', index: 0, input: 'ab', groups: undefined ] [ 'aa', '', index: 0, input: 'aa', groups: undefined ]
"##;
    assert_eq!(run(src), expected.trim());
}

/// Computed field keys evaluate in source order; static initializers see this; private names are own-only; super() from an arrow.
#[test]
fn r2_class_fields_order() {
    let src = r##"
const log = [];
const key = (n) => { log.push('key:' + n); return n; };
class C { [key('a')] = 1; static [key('s')] = 2; [key('b')]() {} static { log.push('blk'); } static [key('t')] = log.push('init t'); get [key('g')]() { return 1; } }
console.log(log.join());
console.log(Object.getOwnPropertyNames(new C()).join(), Object.getOwnPropertyNames(C).join());
class A { static x = 1; static y = this.x + 1; static f = () => this.y; static g() { return this.x; } static { this.z = this.f() + 1; } static #p = this.x + 100; static getP() { return A.#p; } static fn = function () {}; static #pf = () => 1; static pfn() { return A.#pf.name; } }
class B extends A { static x = 10; static { this.w = super.g() + this.z; } static s = super.g(); }
console.log(A.x, A.y, A.f(), A.z, B.x, B.y, B.z, B.w, B.s, A.getP(), A.fn.name, A.pfn());
class P { #x = 1; static read(o) { return o.#x; } static write(o) { o.#x = 2; } static call(o) { return o.#m(); } #m() { return 1; } }
const P1 = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } };
console.log(P1(() => P.read({})), P1(() => P.write({})), P1(() => P.call({})), P1(() => P.read(Object.create(new P()))), P1(() => P.read(null)), P1(() => P.read(new P())), P1(() => P.call(new P())));
const L = [];
class Base { constructor() { L.push('A:' + new.target.name); } }
class Der extends Base { f = L.push('Der.f'); constructor() { const g = () => super(); L.push('pre'); g(); L.push('post'); } }
new Der();
console.log(L.join(' | '));
"##;
    let expected = r##"
key:a,key:s,key:b,key:t,key:g,blk,init t
a length,name,prototype,s,t
1 2 2 3 10 2 3 13 10 101 fn #pf
TypeError: Cannot read private member #x from an object whose class did not declare it TypeError: Cannot write private member #x to an object whose class did not declare it TypeError: Receiver must be an instance of class P TypeError: Cannot read private member #x from an object whose class did not declare it TypeError: Cannot read properties of null (reading '#x') 1 1
pre | A:Der | Der.f | post
"##;
    assert_eq!(run(src), expected.trim());
}

/// try/finally completion values when the finally block breaks.
#[test]
fn r2_completion_values() {
    let src = r##"
for (const src of ['1; do { 2; try { 5; } finally { break; } } while (false)', '1; l: do { 2; try { 5; } finally { break l; } } while (true)',
  '1; do { try { 2; break; } finally { 3; } } while (false)', '1; do { 2; try { break; } finally { 3; } } while (false)',
  '1; for (var i = 0; i < 3; i++) { try { i; } finally { continue; } }', '1; try { 2; } finally { 3; }', 'do { 4; try { 5; } finally { 6; break; } } while (0)'])
  console.log(JSON.stringify(src), (0, eval)(src));
"##;
    let expected = r##"
"1; do { 2; try { 5; } finally { break; } } while (false)" undefined
"1; l: do { 2; try { 5; } finally { break l; } } while (true)" undefined
"1; do { try { 2; break; } finally { 3; } } while (false)" 2
"1; do { 2; try { break; } finally { 3; } } while (false)" undefined
"1; for (var i = 0; i < 3; i++) { try { i; } finally { continue; } }" undefined
"1; try { 2; } finally { 3; }" 2
"do { 4; try { 5; } finally { 6; break; } } while (0)" 6
"##;
    assert_eq!(run(src), expected.trim());
}

/// IteratorClose on throw/break/destructuring, and yield* delegating return/throw.
#[test]
fn r2_iterator_close() {
    let src = r##"
const log = [];
const mk = (n, o = {}) => ({ [Symbol.iterator]() { let i = 0; return { next() { log.push('next' + i); return i < n ? { value: i++, done: false } : { value: undefined, done: true }; }, return(v) { log.push('return'); if (o.throwReturn) throw new Error('rt'); return o.bad ? 1 : { done: true }; } }; } });
let tn = 0;
const T = (f) => { log.length = 0; let r; try { r = f(); } catch (e) { r = 'E:' + e.message; } console.log(++tn, r, log.join(' ')); };
T(() => { for (const x of mk(3)) { throw new Error('body'); } });
T(() => { for (const x of mk(3, { throwReturn: true })) { throw new Error('body'); } });
T(() => { for (const x of mk(3, { throwReturn: true })) { break; } });
T(() => { for (const x of mk(3, { bad: true })) { break; } });
T(() => { const [a] = mk(3); return a; });
T(() => { const [a] = mk(3, { throwReturn: true }); return a; });
T(() => { const [a, b, c, d] = mk(3, { throwReturn: true }); return a; });
T(() => { const [a, ...r] = mk(3, { throwReturn: true }); return a; });
T(() => { const [{ x }] = [null]; });
T(() => { const [a] = mk(1, { bad: true }); return a; });
T(() => { function* g() { try { yield 1; yield 2; } finally { log.push('gfin'); } } const [a] = g(); return a; });
T(() => { function* g() { try { yield 1; } finally { log.push('gfin'); } } for (const x of g()) { throw new Error('b'); } });
T(() => { return Array.from(mk(3), (x) => { if (x == 1) throw new Error('m'); return x; }); });
T(() => { const [x, y] = mk(2, { throwReturn: true }); return x + y; });
T(() => { const it = { [Symbol.iterator]: () => ({}) }; const [a] = it; });
T(() => { const it = { [Symbol.iterator]: () => ({ next: () => 1 }) }; for (const x of it); });
function* g1() { try { yield* mk(3, { throwReturn: false }); } finally { log.push('g-finally'); } }
function* g2() { try { yield* mk(3, { throwReturn: true }); } finally { log.push('g-finally'); } }
for (const [name, g, how] of [['ret', g1, (it) => it.return(9)], ['retThrow', g2, (it) => it.return(9)], ['thr', g1, (it) => it.throw(new Error('t'))], ['thrThrow', g2, (it) => it.throw(new Error('t'))]]) {
  T(() => { log.length = 0; const it = g(); it.next(); return JSON.stringify(how(it)); });
}
T(() => { function* inner() { try { yield 1; yield 2; } catch (e) { yield 'caught ' + e; } } function* outer() { const r = yield* inner(); yield 'r:' + r; } const it = outer(); it.next(); return JSON.stringify([it.throw('X'), it.next(), it.next()]); });
"##;
    let expected = r##"
1 E:body next0 return
2 E:body next0 return
3 E:rt next0 return
4 E:Iterator result 1 is not an object next0 return
5 0 next0 return
6 E:rt next0 return
7 0 next0 next1 next2 next3
8 0 next0 next1 next2 next3
9 E:Cannot read properties of null (reading 'x') 
10 E:Iterator result 1 is not an object next0 return
11 1 gfin
12 E:b gfin
13 E:m next0 next1 return
14 E:rt next0 next1 return
15 E:undefined is not a function 
16 E:Iterator result 1 is not an object 
17 {"done":true} next0 return g-finally
18 E:rt next0 return g-finally
19 E:The iterator does not provide a 'throw' method. next0 return g-finally
20 E:rt next0 return g-finally
21 [{"value":"caught X","done":false},{"value":"r:undefined","done":false},{"done":true}] 
"##;
    assert_eq!(run(src), expected.trim());
}

/// Revoked and non-callable proxies, setPrototypeOf forwarding, for-in and construct trap order.
#[test]
fn r2_proxy_invariants() {
    let src = r##"
const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
const r1 = Proxy.revocable({ a: 1 }, {});
r1.revoke();
console.log(P(() => r1.proxy()), P(() => new r1.proxy()), P(() => typeof r1.proxy), P(() => Array.isArray(r1.proxy)), P(() => Object.prototype.toString.call(r1.proxy)));
const r2 = Proxy.revocable(function () { return 1; }, {});
console.log(P(() => r2.proxy()));
r2.revoke();
console.log(P(() => r2.proxy()), P(() => new r2.proxy()));
const frozen = new Proxy(Object.freeze({ k: 1 }), {});
console.log(P(() => Object.setPrototypeOf(frozen, {})), P(() => Reflect.setPrototypeOf(frozen, {})), P(() => Reflect.setPrototypeOf(frozen, Object.prototype)));
const log = [];
const h = new Proxy({}, { get(_, trap) { log.push(trap); return undefined; } });
const p = new Proxy({ a: 1 }, h);
for (const k in p) {}
try { p(); } catch (e) {}
try { new p(); } catch (e) {}
console.log(log.join());
log.length = 0;
const pf = new Proxy(function () {}, h);
try { new pf(); } catch (e) {}
console.log(log.join(), JSON.stringify(new Proxy(function () {}, {})), JSON.stringify({ f: new Proxy(function () {}, {}) }));
"##;
    let expected = r##"
TypeError: r1.proxy is not a function TypeError: r1.proxy is not a constructor object TypeError: Cannot perform 'IsArray' on a proxy that has been revoked TypeError: Cannot perform 'Object.prototype.toString' on a proxy that has been revoked
1
TypeError: Cannot perform 'apply' on a proxy that has been revoked TypeError: Cannot perform 'construct' on a proxy that has been revoked
TypeError: #<Object> is not extensible false true
ownKeys,getPrototypeOf,getOwnPropertyDescriptor
construct,get undefined {}
"##;
    assert_eq!(run(src), expected.trim());
}

/// toPrimitive method checks, hasInstance, keyFor, RegExp(regexp-like), Object.assign refusals, static call/apply.
#[test]
fn r2_symbol_protocols() {
    let src = r##"
const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const v of [7, 'str', {}, true, 1n, Symbol('q')]) {
  console.log(P(() => +{ [Symbol.toPrimitive]: v }), P(() => new Date({ [Symbol.toPrimitive]: v })));
}
console.log(new Date({ [Symbol.toPrimitive]: () => 5 }).getTime(), new Date({ valueOf() { return 6; } }).getTime(), new Date({ toString() { return '2020-01-01'; } }).getTime(), new Date({ [Symbol.toPrimitive]: () => '2020-01-02' }).getTime());
console.log(P(() => Symbol.keyFor('x')), P(() => Symbol.keyFor(Symbol.for('k'))));
class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }
console.log(P(() => 2 instanceof Even), P(() => 3 instanceof Even), P(() => Function.prototype[Symbol.hasInstance].call(Even, 4)), P(() => Function.prototype[Symbol.hasInstance].call(Even, new Even())), P(() => Function.prototype[Symbol.hasInstance].call({}, {})));
console.log(P(() => ({}) instanceof (() => {})), P(() => ({}) instanceof (async function () {})), P(() => ({ m() {} }).m instanceof Object));
const re = /a/; re[Symbol.match] = true;
console.log(RegExp(re) === re, new RegExp(re) === re, RegExp({ [Symbol.match]: true, source: 'q', flags: 'g', constructor: RegExp }).flags, RegExp(/x/g, 'i').flags);
const m = new Map();
console.log(P(() => Object.assign(m, { [Symbol.toStringTag]: 'X' })), P(() => Object.assign(Object.freeze({}), { a: 1 })), P(() => { 'use strict'; m[Symbol.toStringTag] = 'Y'; }));
class S1 { static call(o) { return 'static call ' + o; } static apply() { return 'static apply'; } static bind() { return 'static bind'; } }
console.log(S1.call(1), S1.apply(), S1.bind());
"##;
    let expected = r##"
TypeError: number 7 is not a function TypeError: '7' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
TypeError: string "str" is not a function TypeError: 'str' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
TypeError: object is not a function TypeError: '#<Object>' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
TypeError: boolean true is not a function TypeError: 'true' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
TypeError: bigint is not a function TypeError: '1' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
TypeError: symbol is not a function TypeError: 'Symbol(q)' returned for property 'Symbol(Symbol.toPrimitive)' of object '#<Object>' is not a function
5 6 1577836800000 1577923200000
TypeError: x is not a symbol k
true false false true false
TypeError: Function has non-object prototype 'undefined' in instanceof check TypeError: Function has non-object prototype 'undefined' in instanceof check true
true false g i
TypeError: Cannot assign to read only property 'Symbol(Symbol.toStringTag)' of object '#<Map>' TypeError: Cannot add property a, object is not extensible TypeError: Cannot assign to read only property 'Symbol(Symbol.toStringTag)' of object '#<Map>'
static call 1 static apply static bind
"##;
    assert_eq!(run(src), expected.trim());
}
