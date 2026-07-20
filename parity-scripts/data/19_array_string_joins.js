// Array <-> string conversions and joins.
const arr = [1, 2, 3, 4, 5];
console.log(arr.join(","));
console.log(arr.join(""));
console.log(arr.join(" - "));
console.log(String(arr));                 // "1,2,3,4,5"
console.log([1, [2, 3], [4, [5]]].join(",")); // flat-join

console.log("a,b,c".split(",").join("|"));
console.log("hello".split("").join("."));
console.log([..."hello"].join(""));
console.log(Array.from("abc").join("-"));

console.log([1, 2, 3].map((n) => n * n).join(","));
console.log("1 2 3 4".split(" ").map(Number).reduce((a, b) => a + b));

console.log(["a", "b", "c"].entries ? [...["a", "b", "c"].entries()].map(([i, v]) => `${i}:${v}`).join(",") : "");
console.log([1, 2, 3, 4].filter((n) => n % 2).join(","));
console.log(Array(5).fill(0).join(","));
console.log(Array.from({ length: 5 }, (_, i) => i).join(","));
console.log([3, 1, 2].concat([6, 4, 5]).sort((a, b) => a - b).join(""));
console.log("csv,data,here".split(",").reverse().join(","));
console.log([null, undefined, 1, 2].join(","));  // empties -> ""
