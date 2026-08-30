// Array methods and higher-order functions.
const nums = [5, 3, 8, 1, 9, 2];
console.log(nums.filter(x => x % 2 === 1).map(x => x * x));
console.log(nums.reduce((a, b) => a + b, 0));
console.log([...nums].sort((a, b) => a - b));
console.log(nums.slice(1, 4), nums.indexOf(8), nums.includes(9));
const [first, second, ...rest] = nums;
console.log(first, second, rest);

// `indexOf`/`lastIndexOf`/`includes` all take a `fromIndex`, and it was being
// ignored outright — `[1,2,3].indexOf(2, 2)` answered 1, reporting a match at
// an index the search was told to start after.
const seek = [1, 2, 3, 2, 1];
console.log("fwd     ", seek.indexOf(2, 2), seek.indexOf(2, 4), seek.indexOf(1, 1));
console.log("back    ", seek.lastIndexOf(2, 2), seek.lastIndexOf(2, 0), seek.lastIndexOf(1));
console.log("has     ", seek.includes(2, 2), seek.includes(2, 4), seek.includes(1, 1));
// Negative counts back from the end and clamps at 0; out of range finds nothing.
console.log("neg     ", seek.indexOf(2, -2), seek.indexOf(2, -99), seek.lastIndexOf(2, -3), seek.lastIndexOf(2, -99));
console.log("range   ", seek.indexOf(1, 99), seek.indexOf(1, 5), [].lastIndexOf(1, 0));
// The forward pair read absent and NaN alike as 0, but `lastIndexOf` must not:
// absent means the last element, while an explicit NaN is ToIntegerOrInfinity'd
// to 0 and so searches index 0 only.
console.log("nan     ", seek.lastIndexOf(1, NaN), seek.lastIndexOf(1), seek.indexOf(1, NaN));
console.log("inf     ", seek.indexOf(1, Infinity), seek.indexOf(1, -Infinity), seek.lastIndexOf(1, Infinity));
// Truncated toward zero, not rounded.
console.log("frac    ", seek.indexOf(2, 1.9), seek.lastIndexOf(2, 3.9));
// A hole is never an `indexOf` match (spec'd through HasProperty) but `includes`
// reads it as undefined; both still honour the start.
const gappy = [1, , 3, , 1];
console.log("holes   ", gappy.indexOf(undefined), gappy.includes(undefined, 1), gappy.indexOf(1, 1));
// Typed arrays carry the same three methods and had the same defect.
const typed = new Uint8Array([1, 2, 3, 2, 1]);
console.log("typed   ", typed.indexOf(2, 2), typed.lastIndexOf(2, 2), typed.includes(2, 4));
