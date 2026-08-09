// Object.freeze / seal / preventExtensions and the sloppy-mode write semantics.
const f = Object.freeze({ a: 1, b: { nested: 2 } });
f.a = 99;
f.c = 3;
console.log(f.a, f.c, Object.isFrozen(f), Object.isSealed(f), Object.isExtensible(f));
// freeze is shallow.
f.b.nested = 20;
console.log(f.b.nested, Object.isFrozen(f.b));

const s = Object.seal({ x: 1 });
s.x = 2;
s.y = 3;
console.log(s.x, s.y, Object.isSealed(s), Object.isFrozen(s), Object.isExtensible(s));

const p = Object.preventExtensions({ k: 1 });
p.k = 2;
p.n = 3;
console.log(p.k, p.n, Object.isExtensible(p), Object.isSealed(p));

console.log(Object.isFrozen(Object.freeze({})), Object.isSealed(Object.seal({})));
console.log(JSON.stringify(Object.keys(f)), JSON.stringify(Object.keys(s)));
