// Object.fromEntries: entry checks, ToPropertyKey, generator sources, iterator
// close on a bad entry, and an own __proto__ key (also from a computed literal).
for (const v of [null, undefined, 5, {}, [1], [[1,2],"ab"], [["a",1],["a",2]], new Map([[{toString(){return "k"}},1]]), [[Symbol.for("s"),3]], (function*(){yield ["g",1]; yield ["h",2]})(), [[1]], [{0:"x",1:"y"}], [["__proto__", 5]]]) {
  try { const o = Object.fromEntries(v); console.log(o, Object.getPrototypeOf(o) === Object.prototype) } catch (e) { console.log(e.constructor.name, e.message) }
}
let closed = 0; const it = { [Symbol.iterator]() { return { next: () => ({ value: 5, done: false }), return() { closed++; return {} } } } };
try { Object.fromEntries(it) } catch (e) { console.log(e.message, closed) }
const o = JSON.parse("{\"__proto__\": 1, \"a\": 2}"); console.log(o, Object.keys(o)); const p={["__proto__"]:3}; console.log(p); console.log({__proto__: null, x: 1})
