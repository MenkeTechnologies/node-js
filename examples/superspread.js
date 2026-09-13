// `super(...args)` and `super.m(...args)` did not spread, the same way
// `new C(...args)` did not: the spread was compiled as an ordinary argument,
// so the parent received the ARRAY itself and its later parameters were
// undefined.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
class Base { constructor(a, b, c) { this.args = [a, b, c]; } m(a, b) { return [a, b]; } }
show("super        ", () => { class D extends Base { constructor() { super(...[1, 2]); } } return new D().args; });
show("super-mixed  ", () => { class D extends Base { constructor() { super(0, ...[1, 2]); } } return new D().args; });
show("super-trailing", () => { class D extends Base { constructor() { super(...[1], 9); } } return new D().args; });
show("super-iterable", () => { class D extends Base { constructor() { super(...new Set([7, 8])); } } return new D().args; });
show("super-empty  ", () => { class D extends Base { constructor() { super(...[]); } } return new D().args; });
show("super-method ", () => { class D extends Base { m() { return super.m(...[3, 4]); } } return new D().m(); });
show("super-m-mixed", () => { class D extends Base { m() { return super.m(0, ...[5]); } } return new D().m(); });
// A native parent sees the spread arguments too.
show("super-native ", () => { class E extends Error { constructor() { super(..."hi"); } } return new E().message; });
show("super-array  ", () => { class A extends Array { constructor() { super(...[3]); } } return new A().length; });
// `this` is still the derived instance, and field initializers still run.
show("super-this   ", () => { class D extends Base { f = "field"; constructor() { super(...[1]); this.after = this.f; } } const d = new D(); return [d.args[0], d.after, d instanceof D]; });

// An OPTIONAL CHAIN cannot be the callee of `new`, nor carry a tagged
// template (13.3.5.1 / 13.3.11.1). Both are early errors; both used to reach
// run time and fail as a TypeError, or silently work.
const attempt = (label, src) => {
  try { eval("class C {}; const a = { b: class {}, t() { return 1; } }; " + src); console.log(label, "ok"); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
attempt("new-opt-call  ", "new C?.()");
attempt("new-opt-member", "new a?.b()");
attempt("tagged-opt    ", "a?.t`t`");
// Parenthesising ENDS the chain, so these are legal.
attempt("new-parens    ", "new (a?.b)()");
attempt("new-then-opt  ", "new C()?.x");
attempt("plain-opt     ", "a?.t()");
attempt("plain-tagged  ", "a.t`t`");

// Tagged templates themselves, pinned so the new rejection did not disturb them.
show("tagged       ", () => { const tag = (s, ...v) => [s.raw.join("|"), v]; return tag`a${1}b${2}c`; });
show("tagged-frozen", () => { const tag = (s) => [Object.isFrozen(s), Object.isFrozen(s.raw)]; return tag`x`; });
show("tagged-cached", () => { const seen = []; const tag = (s) => { seen.push(s); return seen.length > 1 ? seen[0] === seen[1] : null; }; function go() { return tag`same`; } go(); return go(); });
show("tagged-this  ", () => { const o = { tag(s) { return [this === o, s[0]]; } }; return o.tag`q`; });
show("tagged-raw   ", () => { const tag = (s) => [s[0], s.raw[0]]; return tag`a\nb`; });
