// A `catch` parameter can be a PATTERN. Only a bare identifier was handled, so
// the pattern bound nothing and the handler body saw a ReferenceError for every
// name in it — `catch ({ code })` could not read `code` at all.
try { throw { code: "E", msg: "m" }; } catch ({ code, msg }) { console.log("catch-obj  ", code, msg); }
try { throw [1, 2]; } catch ([a, b]) { console.log("catch-arr  ", a, b); }
try { throw { a: { b: 1 } }; } catch ({ a: { b } }) { console.log("catch-deep ", b); }
try { throw {}; } catch ({ z = 9 }) { console.log("catch-dflt ", z); }
try { throw { a: 1, b: 2 }; } catch ({ a, ...rest }) { console.log("catch-rest ", a, JSON.stringify(rest)); }
// The pattern's bindings are scoped to the handler, and a nested throw still
// reaches the outer handler.
try {
  try { throw { n: 1 }; } catch ({ n }) { throw new RangeError("from " + n); }
} catch (e) { console.log("catch-nest ", e.constructor.name, e.message); }

// Destructuring a NULLISH source names the pattern's first property and the
// source expression. It used to report the property read that failed instead.
const t = (f) => { try { f(); return "NO THROW"; } catch (e) { return e.constructor.name + ": " + e.message; } };
const v = null;
const holder = { p: null };
let u;
console.log("literal    ", t(() => { const { w } = null; }));
console.log("undefined  ", t(() => { const { w } = undefined; }));
console.log("variable   ", t(() => { const { w } = v; }));
console.log("undef-var  ", t(() => { const { w } = u; }));
console.log("member     ", t(() => { const { w } = holder.p; }));
console.log("two-keys   ", t(() => { const { a, b } = null; }));
console.log("renamed    ", t(() => { const { a: z } = null; }));
console.log("nested-1st ", t(() => { const { a: { b } } = null; }));
console.log("assign     ", t(() => { let w; ({ w } = null); }));
// No property to name: a rest, a computed key, or an empty pattern.
console.log("rest-only  ", t(() => { const { ...r } = null; }));
console.log("computed   ", t(() => { const { ["k"]: w } = null; }));
console.log("comp-dflt  ", t(() => { const { ["k"]: w = 1 } = null; }));
console.log("empty      ", t(() => { const {} = null; }));
// A first property carrying a DEFAULT reports the READ instead, because the
// default is what performs it.
console.log("first-dflt ", t(() => { const { a = 1 } = null; }));
console.log("later-dflt ", t(() => { const { a, b = 1 } = null; }));
// An array pattern reports iterability, which was already right.
console.log("arr-null   ", t(() => { const [w] = null; }));
console.log("arr-number ", t(() => { const [w] = 5; }));
console.log("arr-object ", t(() => { const [w] = {}; }));

// Everything the guard must not disturb.
const { a = 1, b: { c = 2 } = {}, ...rest } = { a: undefined, d: 4, e: 5 };
console.log("defaults   ", a, c, JSON.stringify(rest));
const { n = 5 } = { n: null };
const [m = 5] = [null];
console.log("null-keeps ", n, m);
const order = [];
const key = () => { order.push("k"); return "q"; };
const { [key()]: qq = (order.push("d"), 9) } = {};
console.log("eval-order ", qq, order.join(","));
const obj = {};
[obj.x, obj.y = 7] = [1];
({ z: obj.z = 8 } = {});
console.log("members    ", JSON.stringify(obj));
function withDefaults(p, q = p * 2, { r = q + 1 } = {}) { return [p, q, r].join(","); }
console.log("params     ", withDefaults(1), withDefaults(1, 10), withDefaults(1, 10, { r: 99 }));
function* counter() { let i = 0; while (true) yield i++; }
const [g1, g2] = counter();
const sym = Symbol("s");
const { ...symRest } = { [sym]: 1, v: 2 };
console.log("misc       ", g1, g2, symRest[sym], symRest.v);
