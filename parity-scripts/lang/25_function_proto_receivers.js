// Function.prototype.call/apply/bind/toString: borrowed and re-applied forms,
// and the IsCallable(this) failure each reports for every receiver type.
function f(a, b) { return [this === undefined ? "u" : typeof this, a, b]; }
console.log(f.call.call(f, 1, 2, 3), Function.prototype.call.call(f, null, 7), f.apply.call(f, 0, [4, 5]), f.bind.call(f, null, 9)(8));
console.log(Function.prototype.apply.call(Math.max, null, [1, 5, 2]), Reflect.apply(Function.prototype.call, f, [null, 1]));
const call = Function.prototype.call; const bound = call.bind(Array.prototype.slice); console.log(bound([1, 2, 3], 1));
const uncurry = (fn) => (...a) => call.apply(fn, a); console.log(uncurry(String.prototype.toUpperCase)("abc"), uncurry([].join)([1, 2], "-"));
try { Function.prototype.call.call(1) } catch (e) { console.log(e.name, e.message) }
try { Function.prototype.apply.call({}) } catch (e) { console.log(e.name, e.message) }
try { Function.prototype.bind.call("s") } catch (e) { console.log(e.name, e.message) }
try { f.apply(null, 5) } catch (e) { console.log(e.name, e.message) }
console.log(f.apply(null, { length: 2, 0: "x", 1: "y" }), f.apply(null, null), f.apply(null));
console.log(Function.prototype.toString.call(f).length, Function.prototype.toString.call(Math.max), typeof Function.prototype, Function.prototype());
try { Function.prototype.toString.call({}) } catch (e) { console.log(e.name, e.message) }
console.log(f.bind(null, 1).name, f.bind(null, 1).length, f.bind().bind().name, (() => {}).bind().name);
for (const r of [1, "s", true, Symbol("q"), 1n, {}, [], [1, 2], null, undefined, new Map(), /x/, Object.create(null), new (class K {})()]) {
  for (const m of ["call", "apply", "bind", "toString"]) {
    try { Function.prototype[m].call(r); } catch (e) { console.log(m, e.name, e.message); }
  }
}
