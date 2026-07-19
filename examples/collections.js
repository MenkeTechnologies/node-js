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
