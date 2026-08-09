// structuredClone: deep copy, aliasing, and the types it preserves.
const src = { n: 1, s: "x", b: true, nul: null, arr: [1, [2, 3]], nested: { deep: { k: "v" } } };
const c = structuredClone(src);
console.log(JSON.stringify(c));
console.log(c !== src, c.nested !== src.nested, c.arr[1] !== src.arr[1]);
c.nested.deep.k = "changed";
console.log(src.nested.deep.k, c.nested.deep.k);

// Shared references inside one input stay shared in the clone.
const shared = { id: 1 };
const withShared = { a: shared, b: shared };
const cs = structuredClone(withShared);
console.log(cs.a === cs.b, cs.a !== shared);

// Cycles survive.
const cyc = { name: "root" };
cyc.self = cyc;
const cc = structuredClone(cyc);
console.log(cc.self === cc, cc.name);

// Map/Set/Date/RegExp are structured types, not plain objects.
const m = structuredClone(new Map([["k", [1, 2]]]));
console.log(m instanceof Map, m.get("k").join(","));
const st = structuredClone(new Set([1, 2, 2, 3]));
console.log(st instanceof Set, st.size, JSON.stringify([...st]));
const d = structuredClone(new Date(0));
console.log(d instanceof Date, d.toISOString());
const re = structuredClone(/ab+c/gi);
console.log(re instanceof RegExp, re.source, re.flags);
console.log(structuredClone(10n) === 10n, typeof structuredClone(undefined));
const ta = structuredClone(new Uint8Array([1, 2, 3]));
console.log(ta instanceof Uint8Array, ta.length, ta[2]);
