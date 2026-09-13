// Strict mode: the early errors and the runtime restrictions. A top-level
// `'use strict'` never reached the runtime frame until the previous commit, and
// these five restrictions were not implemented at all - so a strict file
// behaved as sloppy code in every way except the assignment-to-undeclared
// check, which the compiler emitted directly.
"use strict";
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// `delete` of a plain name is an early error, whatever the name is bound to.
show("delete-name  ", () => eval("var dv = 1; delete dv"));
show("delete-param ", () => eval("(function (a) { delete a })"));
// …but a PROPERTY delete is fine, and so is the sloppy form.
show("delete-prop  ", () => { const o = { a: 1 }; return [delete o.a, delete o.nope]; });

// A duplicate parameter name is refused; sloppy code keeps the LAST one.
show("dup-param    ", () => eval("function f(a, a) {}"));
show("dup-arrow    ", () => eval("(a, a) => 1"));
show("dup-pattern  ", () => eval("function f({ a }, [a]) {}"));

// `eval` and `arguments` may not be bound, assigned or updated.
for (const src of ["eval = 1", "arguments = 1", "var eval = 1", "let arguments = 1",
  "function f(eval) {}", "eval++", "--arguments", "({ a: eval } = {})"]) {
  show(("reserved " + JSON.stringify(src)).padEnd(34), () => eval(src));
}
// A property of those names is untouched, and so is a plain READ.
show("as-property  ", () => { const o = { eval: 1, arguments: 2 }; return [o.eval, o.arguments]; });
show("read         ", () => [typeof eval, (function () { return typeof arguments; })()]);

// `arguments.callee` is a POISON PILL in strict code: the accessor throws
// rather than answering, which is how a strict function keeps its caller
// unreachable. It read back as `undefined`, which a probe reads as
// "not supported" rather than "forbidden".
show("callee       ", () => { function f() { return arguments.callee; } return f(); });
// Only `callee` is poisoned on an ARGUMENTS object — `caller` is simply absent
// there. On a strict FUNCTION both `caller` and `arguments` throw.
show("args-caller  ", () => { function f() { return arguments.caller; } return f(); });
show("fn-caller    ", () => { function f() { return f.caller; } return f(); });
show("fn-arguments ", () => { function f() { return f.arguments; } return f(); });
show("indices      ", () => { function f() { return [arguments[0], arguments.length]; } return f(7); });

// The runtime restrictions the frame flag drives.
show("this         ", () => { function f() { return this; } return f() === undefined; });
show("detached     ", () => { const o = { m() { return this; } }; const g = o.m; return [o.m() === o, g() === undefined]; });
show("frozen       ", () => { const o = Object.freeze({ a: 1 }); o.a = 2; return "no throw"; });
show("readonly-prot", () => { const c = Object.create(Object.freeze({ a: 1 })); c.a = 2; return "no throw"; });
show("getter-only  ", () => { const o = { get g() { return 1; } }; o.g = 2; return "no throw"; });
show("non-extensibl", () => { const o = Object.preventExtensions({}); o.x = 1; return "no throw"; });
show("undeclared   ", () => { undeclaredNameHere = 1; return "no throw"; });

// A LEGACY OCTAL literal is base 8 in sloppy code - `012` is 10, not 12,
// which was read as decimal. A run containing an 8 or a 9 stays decimal.
// (Both forms are SyntaxErrors in strict code, which this lexer cannot see;
// recorded in BUGS.md.)
// An INDIRECT eval is global-scope SLOPPY code, whatever the caller is.
const indirect = eval;
console.log("octal        ", JSON.stringify(indirect("[012, 0o12, 08, 09, 0888, 00, 007, 0777, 0.5, 0e1, 0b11]")));
