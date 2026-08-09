// Prototype identity, chain walking, and for-in over inherited enumerables.
const base = { inherited: 1 };
const derived = Object.create(base);
derived.own = 2;
const seen = [];
for (const k in derived) seen.push(k);
console.log(JSON.stringify(seen), JSON.stringify(Object.keys(derived)));
console.log(Object.getPrototypeOf(derived) === base, base.isPrototypeOf(derived));

function Legacy() { this.field = 1; }
Legacy.prototype.method = function () { return "m"; };
Legacy.prototype.data = 5;
const l = new Legacy();
const lk = [];
for (const k in l) lk.push(k);
console.log(JSON.stringify(lk.sort()), l.method(), l.data);
console.log(Object.getPrototypeOf(l) === Legacy.prototype);
console.log(l.constructor === Legacy, Legacy.prototype.constructor === Legacy);
console.log(JSON.stringify(Object.keys(Legacy.prototype)));

class A { m() { return "A"; } get g() { return 1; } }
class B extends A { n() { return "B"; } }
const b = new B();
b.self = 1;
const bk = [];
for (const k in b) bk.push(k);
console.log(JSON.stringify(bk), b.m(), b.n(), b.g);
console.log(Object.getPrototypeOf(B.prototype) === A.prototype);
console.log(Object.getPrototypeOf(A.prototype) === Object.prototype);
console.log(JSON.stringify(Object.getOwnPropertyNames(A.prototype).sort()));

const nul = Object.create(null);
nul.k = 1;
console.log(Object.getPrototypeOf(nul), JSON.stringify(Object.keys(nul)));

console.log(Math === Math, JSON === JSON, Array.prototype === Array.prototype);
console.log(Object.getPrototypeOf([]) === Array.prototype, Object.getPrototypeOf({}) === Object.prototype);
