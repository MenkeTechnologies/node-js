// Sorting numbers (comparator) & strings.
const nums = [10, 2, 33, 4, 100, 5, 1];
console.log([...nums].sort());                       // lexicographic default
console.log([...nums].sort((a, b) => a - b));        // ascending numeric
console.log([...nums].sort((a, b) => b - a));        // descending numeric

const strs = ["banana", "Apple", "cherry", "apple", "Banana"];
console.log([...strs].sort());
console.log([...strs].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase())));

const words = ["ccc", "a", "bbbb", "dd"];
console.log([...words].sort((a, b) => a.length - b.length));

const objs = [{ n: "x", v: 3 }, { n: "y", v: 1 }, { n: "z", v: 2 }];
console.log(objs.sort((a, b) => a.v - b.v).map((o) => o.n).join(","));

// stable sort check
const pairs = [[1, "a"], [1, "b"], [0, "c"], [1, "d"], [0, "e"]];
console.log(pairs.sort((a, b) => a[0] - b[0]).map((p) => p[1]).join(""));

console.log([3.3, 1.1, 2.2].sort((a, b) => a - b));
console.log([-5, 3, -1, 0, 2].sort((a, b) => a - b));
console.log(["10", "9", "100", "1"].sort());
