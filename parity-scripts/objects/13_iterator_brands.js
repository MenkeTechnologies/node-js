// Array and Map/Set iterators carry their prototypes' Symbol.toStringTag, and
// util.inspect shows a live Map/Set iterator's remaining entries as node's
// formatIterator does. An array iterator printed `undefined` and a Map/Set
// one `CollectionIterator {}`.
const m = new Map([[1, 'a'], [2, 'b']]);
const s = new Set([1, 2]);
const it = m.entries();
it.next();
console.log(m.entries(), m.keys(), m.values(), s.values(), s.entries(), it, new Map().keys());
console.log([1, 2].values(), [1].keys(), [1].entries(), { x: [1].values() });
console.log(Object.prototype.toString.call(m.keys()), Object.prototype.toString.call(s.entries()), Object.prototype.toString.call([].values()), String([].values()));
console.log([].values()[Symbol.toStringTag], m.keys()[Symbol.toStringTag], new Set().entries()[Symbol.toStringTag]);
console.log({ a: { b: { c: new Map([[1, 2]]).entries() } } }, { a: { b: { c: [1].values() } } });
console.log(new Set().values(), [new Map([[{ a: 1 }, [1, 2]]]).entries()]);
const big = new Set();
for (let i = 0; i < 103; i++) big.add(i);
console.log(big.values());
const drained = s.values();
for (const _ of drained);
console.log(drained, [...m.keys()], Array.from(m.values()));
