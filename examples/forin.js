// `for-in` enumerates LAZILY: a key the body deletes before the loop reaches it
// is never visited. The key list was taken once up front and every key in it was
// visited regardless, so `delete d.z` inside the loop still produced `x,y,z`.
const d = { x: 1, y: 2, z: 3 };
const got = [];
for (const k in d) { got.push(k); delete d.z; }
console.log("deleted ", got.join(","));
// …including a key deleted from the PROTOTYPE half of the chain.
const base = { z: 9 };
const o = Object.create(base);
o.a = 1;
const seen = [];
for (const k in o) { seen.push(k); delete base.z; }
console.log("proto   ", seen.join(","));
// A key ADDED mid-loop is not visited, which the snapshot already got right.
const q = { a: 1 };
const added = [];
for (const k in q) { added.push(k); q.b = 2; }
console.log("added   ", added.join(","));
// Deleting the CURRENT key must not skip the next one.
const dd = { p: 1, q: 2, r: 3 };
const self = [];
for (const k in dd) { self.push(k); delete dd[k]; }
console.log("current ", self.join(","));

// The re-check is EXISTENCE, not enumerability: making a key non-enumerable
// mid-loop still visits it. Node does not re-filter on `enumerable` here, and
// re-deriving the enumerable set per key dropped `c`.
const fl = { a: 1, b: 2, c: 3 };
const flip = [];
for (const k in fl) { flip.push(k); Object.defineProperty(fl, "c", { enumerable: false }); }
console.log("flip    ", flip.join(","));

// On a Proxy the per-key check IS the `getOwnPropertyDescriptor` trap, so the
// traps interleave with the body — they used to all fire before the first
// visit, because the key list was filtered eagerly. `has` is never called.
const log = [];
const p = new Proxy({ a: 1, b: 2 }, {
  ownKeys(t) { log.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) { log.push("gopd:" + k); return Reflect.getOwnPropertyDescriptor(t, k); },
  has(t, k) { log.push("has:" + k); return Reflect.has(t, k); },
});
for (const k in p) log.push("visit:" + k);
console.log("traps   ", log.join(" "));
// A proxy key hidden by the trap mid-loop is skipped — there the enumerability
// DOES get re-read, because it comes from the trap node itself calls.
let hide = false;
const t2 = { a: 1, b: 2, c: 3 };
const p2 = new Proxy(t2, {
  getOwnPropertyDescriptor(o2, k) {
    const desc = Reflect.getOwnPropertyDescriptor(o2, k);
    return hide && k === "c" ? { ...desc, enumerable: false } : desc;
  },
});
const hidden = [];
for (const k in p2) { hidden.push(k); hide = true; }
console.log("pxflip  ", hidden.join(","));

// Everything the guard must leave alone.
const arr = [10, 20, 30];
const ai = [];
for (const i in arr) ai.push(i);
const si = [];
for (const i in "abc") si.push(i);
console.log("indexed ", ai.join(","), si.join(","));
for (const k in null) console.log("unreachable");
for (const k in 5) console.log("unreachable");
console.log("nullish  ok");
const shadowed = Object.create({ a: 1, b: 2 });
shadowed.a = 9;
const sh = [];
for (const k in shadowed) sh.push(k + "=" + shadowed[k]);
console.log("shadow  ", sh.join(","));
const ne = {};
Object.defineProperty(ne, "hidden", { value: 1, enumerable: false });
ne.shown = 2;
const nek = [];
for (const k in ne) nek.push(k);
console.log("nonenum ", nek.join(","));
const lbl = [];
outer: for (const k in { a: 1, b: 2, c: 3 }) {
  if (k === "b") continue outer;
  if (k === "c") break outer;
  lbl.push(k);
}
console.log("labels  ", lbl.join(","));
const nest = [];
for (const i in { x: 1, y: 2 }) for (const j in { x: 1, y: 2 }) nest.push(i + j);
console.log("nested  ", nest.join(","));
const mixed = {};
mixed.b = 1; mixed[2] = 2; mixed.a = 3; mixed[1] = 4;
const ord = [];
for (const k in mixed) ord.push(k);
console.log("order   ", ord.join(","));
