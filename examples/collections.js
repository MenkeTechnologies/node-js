// Map / Set / Symbol and the iterator protocol.
const scores = new Map([
  ["alice", 90],
  ["bob", 75],
]);
scores.set("carol", 88);
console.log("size", scores.size);
console.log("get bob", scores.get("bob"));
console.log("has dave", scores.has("dave"));
console.log("keys", [...scores.keys()]);
console.log("values", [...scores.values()]);

let report = [];
scores.forEach((v, k) => report.push(`${k}=${v}`));
console.log("forEach", report.join(","));

const tags = new Set();
for (const t of ["a", "b", "a", "c", "b"]) {
  tags.add(t);
}
console.log("unique", [...tags], "count", tags.size);
console.log("set has b", tags.has("b"));
tags.delete("b");
console.log("after delete", [...tags]);

const sym = Symbol("id");
console.log("typeof", typeof sym, "desc", sym.description);
console.log("interned", Symbol.for("x") === Symbol.for("x"));

const collection = {
  items: [10, 20, 30],
  [Symbol.iterator]() {
    let i = 0;
    const items = this.items;
    return {
      next() {
        return i < items.length
          ? { value: items[i++], done: false }
          : { value: undefined, done: true };
      },
    };
  },
};
console.log("custom iterable", [...collection]);
console.log("sum", [...collection].reduce((a, b) => a + b, 0));

// A `Map`/`Set` iterator is LIVE: it sees the collection as it is at each step.
// An entry added during iteration IS visited, and one deleted before it is
// reached is NOT. Every entry used to be materialized up front, so a loop that
// deletes as it goes still processed the entries it had removed.
const walk = (mutate) => {
  const m = new Map([[1, "a"], [2, "b"], [3, "c"]]);
  const seen = [];
  for (const [k] of m) { seen.push(k); mutate(m, k); }
  return seen.join(",");
};
console.log("add     ", walk((m, k) => { if (k === 1) m.set(4, "d"); }));
console.log("del-next", walk((m, k) => { if (k === 1) m.delete(2); }));
console.log("del-self", walk((m, k) => { if (k === 2) m.delete(2); }));
console.log("del-back", walk((m, k) => { if (k === 2) m.delete(1); }));
console.log("del-rest", walk((m, k) => { if (k === 1) { m.delete(2); m.delete(3); } }));
console.log("clear   ", walk((m, k) => { if (k === 1) m.clear(); }));
const walkSet = (mutate) => {
  const s = new Set([1, 2, 3]);
  const seen = [];
  for (const v of s) { seen.push(v); mutate(s, v); }
  return seen.join(",");
};
console.log("set-add ", walkSet((s, v) => { if (v === 1) s.add(4); }));
console.log("set-del ", walkSet((s, v) => { if (v === 2) s.delete(1); }));

// Everything the iterator is used through still behaves: spread, destructuring,
// Array.from, nested loops over one collection, partial consumption, and
// exhaustion.
const pairs = new Map([[1, "a"], [2, "b"]]);
console.log("views   ", [...pairs.keys()].join(","), [...pairs.values()].join(","), [...pairs.entries()].map((e) => e.join(":")).join(","));
console.log("spread  ", [...pairs].map((e) => e.join(":")).join(","), Array.from(pairs.keys()).join(","));
console.log("destr   ", (() => { const [[k, v]] = pairs; return k + "=" + v; })());
console.log("nested  ", (() => { const o = []; for (const [a] of pairs) for (const [b] of pairs) o.push(`${a}${b}`); return o.join(","); })());
console.log("partial ", (() => { const it = pairs.keys(); it.next(); return [...it].join(","); })());
console.log("done    ", (() => { const it = pairs.keys(); [...it]; const r = it.next(); return [r.done, String(r.value)].join(","); })());
console.log("protocol", (() => { const it = pairs.keys(); return [typeof it.next, it[Symbol.iterator]() === it].join(","); })());
const letters = new Set(["x", "y"]);
console.log("set-view", [...letters].join(","), [...letters.keys()].join(","), [...letters.entries()].map((e) => e.join(":")).join(","));
console.log("empty   ", [...new Map()].length, [...new Set()].length);
