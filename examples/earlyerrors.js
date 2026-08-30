// Duplicate lexical declarations are a SyntaxError before anything runs. This
// engine accepted them and let the second declaration win — the gap that lets a
// genuine double-declaration bug through silently, and one that bit three test
// files in this repo whose collisions node rejected and this did not.
//
// Each snippet is compiled in a CHILD, since an early error cannot be caught by
// the program containing it. `stdio` is passed explicitly so the child's stderr
// is captured rather than echoed here.
const { execFileSync } = require("child_process");

const compile = (src) => {
  try {
    execFileSync(process.argv[0], ["-e", src], { stdio: ["ignore", "pipe", "pipe"] });
    return "accepted";
  } catch (e) {
    const m = String(e.stderr).match(/SyntaxError: [^\n]*/);
    return m ? m[0].replace("SyntaxError: ", "") : "other:" + e.status;
  }
};

// Rejected: two lexical declarations of one name in the same scope.
for (const src of [
  "let a=1; let a=2;",
  "const b=1; const b=2;",
  "class G{}; class G{}",
  "let [k]=[1]; const k=2;",
  "const {j}={j:1}; let j=2;",
]) console.log("dup     ", compile(src));

// Rejected: a `var` hoisting onto a lexical name, from however deep.
for (const src of [
  "const c=1; var c=2;",
  "{ var e=1; } let e=2;",
  "let f=1; if(1){ var f=2; }",
]) console.log("var     ", compile(src));

// Rejected: a function declaration beside a lexical name, either order.
for (const src of ["function g(){} let g=1;", "let h=1; function h(){}"]) console.log("func    ", compile(src));

// Rejected: switch cases share ONE block scope.
console.log("switch  ", compile("switch(1){case 1: let i=1; break; case 2: let i=2;}"));

// Accepted: repeated `var`, separate scopes, and everything that only LOOKS
// like a collision. A false positive here rejects a working program, so these
// matter as much as the rejections.
for (const src of [
  "var l=1; var l=2;",
  "let m=1; { let m=2; }",
  "let n=1; function s(){ let n=2; }",
  "for(let o of [1]){} for(let o of [2]){}",
  "try{}catch(p){ let q=1; }",
  "switch(1){case 1: let r=1; break; case 2: let t=2;}",
  "function u(){} function u(){}",
  "let v=1; { function v(){} }",
  "const {w}={w:1}; let x=2;",
]) console.log("ok      ", compile(src));

// The two child_process defects this file's own harness exposed:
// `execFileSync` threw a bare message, so `e.stderr` above was undefined; and
// the stderr echo fired even when `stdio` was given, which node's default-only
// rule suppresses.
try {
  execFileSync(process.argv[0], ["-e", "process.exit(7)"], { stdio: ["ignore", "pipe", "pipe"] });
} catch (e) {
  console.log("errshape", e instanceof Error, e.status, typeof e.stderr, typeof e.stdout);
}

// 15.7.1: a private name must be declared by an enclosing class body. An
// undeclared one is a SyntaxError at PARSE time — it used to parse fine and
// throw a TypeError only if the read ever ran, so a typo in a branch that never
// executed shipped silently.
const syn = (src) => { try { eval(src); return "parsed"; } catch (e) { return e.constructor.name + ": " + e.message; } };
console.log("bare    ", syn("this.#x"));
console.log("in-fn   ", syn("function f() { return this.#y; }"));
console.log("in-class", syn("class C { m() { return this.#z; } }"));
console.log("obj     ", syn("const o = {}; o.#w"));
// A declaration BELOW the use in the same body is still in scope.
console.log("hoisted ", syn("class C { m() { return this.#later; } #later = 1; }"));
// A nested class may reference a name an OUTER class declares.
console.log("nested  ", syn("class B { #v = 1; f() { const s = this; return class { g() { return s.#v; } }; } }"));
// A private method, a private static and a brand check all count.
console.log("kinds   ", syn("class D { #m() {} static #s = 1; run() { return this.#m(); } }"),
  syn("class E { #b = 1; static has(o) { return #b in o; } }"));
// The undeclared name is named in the message, and the innermost class does not
// silently satisfy a use of a name only an unrelated class declares.
console.log("unrelated", syn("class F { #a = 1; } class G { m() { return this.#a; } }"));
