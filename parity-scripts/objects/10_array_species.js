// ArraySpeciesCreate (23.1.3.4): a subclass is its own species unless its
// `Symbol.species` getter says otherwise; `undefined` and `null` both mean a
// plain Array, and a getter that throws propagates out of the method.
const show = (d) => [d instanceof Array, d.constructor.name, JSON.stringify(d)].join(' ');
class A extends Array {}
class U extends Array { static get [Symbol.species]() { return undefined; } }
class N extends Array { static get [Symbol.species]() { return null; } }
class P extends Array { static get [Symbol.species]() { return Array; } }
class T extends Array { static get [Symbol.species]() { throw new Error('species boom'); } }
for (const C of [A, U, N, P]) {
  const m = C.from([3, 1, 2]);
  console.log(C.name, show(m.map((x) => x * 2)), show(m.filter((x) => x > 1)), show(m.slice(1)), show(m.concat([4])), show(m.flat()), show(m.splice(0, 1)));
}
try { T.from([1]).map((x) => x); } catch (e) { console.log(e.message); }
function Legacy() {}
Legacy.prototype = Object.create(Array.prototype);
Legacy.prototype.constructor = Legacy;
const arr = [1, 2]; Object.setPrototypeOf(arr, Legacy.prototype);
console.log(show(arr.map((x) => x)));
