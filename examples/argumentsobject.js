// The `arguments` exotic. Its indices are backed by an ordinary array here, so
// the parts that depend on that representation were wrong: `length` grew when
// an index past the end was written, and `callee` — the pre-`class`
// self-reference idiom — read back `undefined` in sloppy code.
//
// What is NOT pinned here is the sloppy MAPPING between an index and its
// parameter; see BUGS.md. The three lines below that would show it are left
// out rather than pinned wrong.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// `callee` is the running function in sloppy code, and a poison pill in strict.
function self1(a) { return [arguments.callee === self1, arguments.callee.name]; }
show("callee       ", () => self1(1));
const self2 = function named(a) { return [arguments.callee === self2, arguments.callee.name]; };
show("callee-expr  ", () => self2(1));
function self3(a) { "use strict"; try { return arguments.callee; } catch (e) { return e.constructor.name; } }
show("callee-strict", () => self3(1));
// An index PAST the end is an ordinary own property: it reads back, it
// enumerates, and it does NOT move `length`.
function past(a, b) { arguments[1] = 9; return [a, b, arguments.length, arguments[1]]; }
show("past-end     ", () => past(1));
function far(a) { arguments[5] = 7; return [arguments.length, arguments[5], Object.keys(arguments)]; }
show("far-past     ", () => far(1));
// `length` is a plain data property (10.4.4.6) — configurable, unlike an
// array's, which the shared backing made it report as not.
function shape(a) { return [Object.getOwnPropertyDescriptor(arguments, "length"), Object.getOwnPropertyDescriptor(arguments, "0")]; }
show("descriptors  ", () => shape(1));
// It is not an Array, and does not inherit `Array.prototype`.
function kind(a) { return [Array.isArray(arguments), Object.prototype.toString.call(arguments), typeof arguments.map, typeof arguments.hasOwnProperty]; }
show("not-an-array ", () => kind(1));
// …but it IS iterable, and spreads.
function iter(a, b) { return [typeof arguments[Symbol.iterator], [...arguments], Array.from(arguments), Object.keys(arguments)]; }
show("iterable     ", () => iter(1, 2));
show("empty        ", () => (function () { return [arguments.length, [...arguments]]; })());
// An arrow has no `arguments` of its own — it sees the enclosing function's.
function outer(a) { const inner = () => [...arguments]; return inner(); }
show("arrow-lexical", () => outer(1, 2));
// `delete` removes the index, and the unmapped forms — a default, a rest, a
// destructured parameter — leave the two views independent in node too.
function del(a) { delete arguments[0]; return [a, arguments[0], 0 in arguments, arguments.length]; }
show("delete       ", () => del(1));
function dflt(a = 1) { a = 99; return [arguments[0], a]; }
show("default      ", () => dflt(5));
function rest(...r) { r[0] = 99; return [arguments[0], r[0]]; }
show("rest         ", () => rest(5));
function destr({ a }) { return [JSON.stringify(arguments[0]), a]; }
show("destructured ", () => destr({ a: 1 }));
function strictfn(a) { "use strict"; a = 99; return [arguments[0], a]; }
show("strict       ", () => strictfn(1));
