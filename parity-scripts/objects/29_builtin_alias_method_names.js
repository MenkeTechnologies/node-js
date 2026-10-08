// A prototype slot that holds another method's function object runs, and fails
// its receiver check, under that method's name: Map.prototype[Symbol.iterator]
// is `entries`, Set.prototype.keys and [Symbol.iterator] are `values`.
const t = (s) => {
  try {
    console.log(s, String(eval(s)));
  } catch (e) {
    console.log(s, e.constructor.name + ": " + e.message);
  }
};
for (const s of [
  "Map.prototype[Symbol.iterator] === Map.prototype.entries",
  "Set.prototype[Symbol.iterator] === Set.prototype.values",
  "Set.prototype.keys === Set.prototype.values",
  "Map.prototype[Symbol.iterator].name",
  "Set.prototype.keys.name",
])
  t(s);
for (const r of ["null", "1", "{}"])
  for (const m of [
    "Set.prototype.keys",
    "Set.prototype[Symbol.iterator]",
    "Map.prototype[Symbol.iterator]",
    "String.prototype[Symbol.iterator]",
    "Set.prototype.values",
    "Map.prototype.entries",
  ])
    t(m + ".call(" + r + ")");
console.log([...Set.prototype[Symbol.iterator].call(new Set([1, 2]))], [...Map.prototype[Symbol.iterator].call(new Map([[1, 2]]))]);
