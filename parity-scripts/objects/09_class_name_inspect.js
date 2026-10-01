// util.inspect names a class the way it names a function: an anonymous class
// expression takes its inferred binding name, and one with no name at all
// prints `(anonymous)`.
console.log(class {}, class extends Map {}, class extends null {});
const X = class {};
let Z; Z = class {};
const o = { K: class { constructor() { this.a = 1; } }, ['c' + 'd']: class {} };
console.log(X, Z, o, new X(), new o.K());
// The four-wide array wraps only because `[class (anonymous)]` is long enough.
console.log([function () {}, class {}, Symbol.iterator, 10n]);
// An instance of a builtin-collection subclass leads with its constructor and
// the builtin tag, at every nesting depth and past the depth limit.
class M2 extends Map {}
class S2 extends Set {}
class P2 extends Promise {}
const m = new M2();
m.set(1, { a: new M2([[2, 3]]) });
console.log(m, [new M2()], new S2(), new S2([1, 2]));
console.log({ a: { b: { c: new M2([[1, 2]]), d: new S2([1]) } } });
console.log(P2.resolve(3), new P2(() => {}), new Map(), new Set([1]));
const rejected = P2.reject(new RangeError('no'));
rejected.catch(() => {});
console.log(rejected instanceof P2);
