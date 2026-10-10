// Early errors and the wording V8 gives each. A program that is a SyntaxError
// dies before it prints, so each snippet goes through `eval` and the message is
// the observation. Valid siblings sit among the invalid ones: a parser that
// rejects too much is as wrong as one that accepts too much.
const cases = [
  // Token-class messages.
  "var 1", "f(1 2)", "f(", "a b", "let x = ;", "1 +", "x = {a b}", "[1 2]", "if (",
  "for (;;", "else", "var enum", "if (1) else 2", "x = 'a' 'b'", "a = 1 2", ")", "}",
  "'abc", "/abc", "@", "var a = {1 2}", "class { }", "class extends A {}",
  "function () {}", "export default 1", "import x from 'y'",
  // Labels, jumps, returns.
  "a: a: 1", "a: { a: 1 }", "a: { (function () { a: 1 })() }", "break", "continue",
  "a: { continue a }", "a: { break b }", "for (;;) { continue b }", "return 1",
  "(function () { return 1 })()",
  // ASI-sensitive forms.
  "do ; while (0) 1", "do x; while (0) y", "throw\n1", "var a = 1\n/1/g.test('1')",
  "function* g() { yield\n* 2 }", "x\n++\ny",
  // Assignment targets.
  "1 = 2", "a + b = 1", "this = 1", "x?.y = 1", "x?.y++", "++1", "1++",
  "({ a: 1 }) = 1", "([a]) = [1]", "(a) = 1", "(a.b) = {}",
  "[a, ...b, c] = []", "({ ...a, b } = {})", "const [a, ...b, c] = []",
  "function f() {} f() = 1", "function f() {} f()++", "function f() {} for (f() of []) ;",
  // Declarations.
  "const x", "const [a]", "let [a]", "let let = 1", "for (let i, j of []) ;",
  "for (const x = 1 of []) ;", "for (let i of [1]) { var i }", "for (var i of [1]) { var i }",
  "if (1) const x = 1", "if (1) class C {}", "while (0) function f() {}",
  "if (1) function f() {}", "label: function f() {}",
  // Classes and accessors.
  "class A { constructor() {} constructor() {} }", "class A { get a(x) {} }",
  "class A { set a() {} }", "({ get a(x) {} })", "({ set a(...x) {} })",
  "class A { #a; #a }", "class A { get #a() {} set #a(v) {} }",
  "({ __proto__: 1, __proto__: 2 })", "({ __proto__: 1, ['__proto__']: 2 })",
  "({ a = 1 })", "({ a = 1 } = {})", "for ({ a = 1 } of [{}]) ;", "[{ a = 1 }]",
  "({ a = 1 }) => a",
  // Parameters.
  "function f(a, a) {}", "function f(a, a) { 'use strict' }", "(a, a) => 1",
  "({ m(a, a) {} })", "function f(a = 1) { 'use strict' }", "function f(eval) { 'use strict' }",
  // Strict-only words and literals.
  "'use strict'; var static", "'use strict'; yield 1", "'use strict'; 010",
  "'use strict'; '\\101'", "'use strict'; 08", "010", "'\\101'",
  // super, await, ??
  "super.x", "function w() { super.x }", "await 1", "async () => await",
  "a ?? b || c", "a || b ?? c", "(a || b) ?? c", "a ?? (b || c)", "a && b ?? c",
  "-1 ** 2", "(-1) ** 2", "typeof a ** 2",
  // Templates and regex literals.
  "`${}`", "`a${1`", "/(/", "/a/gg", "/[b-a]/", "/\\p{Foo}/u", "/a{2,1}/", "/(?<a>.)(?<a>.)/",
  "if (0) { /(/ }",
  // Optional chains.
  "a?.`x`", "a?.b`x`", "new a?.b()",
];
for (const src of cases) {
  let r;
  try { r = "ok:" + typeof eval(src); } catch (e) { r = e.name + ": " + e.message; }
  console.log(JSON.stringify(src), "=>", r);
}
