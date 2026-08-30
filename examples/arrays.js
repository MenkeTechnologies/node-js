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

// `sort` with CONSISTENT comparators, which is the only case the spec pins:
// 23.1.3.30 leaves the order implementation-defined when the comparator is
// inconsistent, so a constant `() => 1` is deliberately not asserted here —
// node and this engine genuinely differ there and neither is wrong.
let sortSeed = 12345;
const nextRandom = () => (sortSeed = (sortSeed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
const sample = (n) => Array.from({ length: n }, () => Math.floor(nextRandom() * 50));
for (const n of [0, 1, 2, 3, 5, 33, 257]) {
  const data = sample(n);
  console.log("sort" + String(n).padEnd(4),
    data.slice().sort((a, b) => a - b).join(","),
    data.slice().sort().join(","));
}
// Stability: many ties, compared by the ORIGINAL index so a reordering of
// equal elements shows up.
const tied = sample(60).map((v, i) => ({ key: v % 4, i }));
console.log("stable  ", tied.sort((a, b) => a.key - b.key).map((o) => o.i).join(","));
// Sorting is in place and returns the same array; `toSorted` is neither.
const inPlace = [2, 1];
const returned = inPlace.sort();
const copied = [2, 1];
const fresh = copied.toSorted();
console.log("inplace ", returned === inPlace, inPlace.join(","), fresh === copied, copied.join(","), fresh.join(","));
// Holes and undefined both sort to the end, undefined ahead of the holes.
console.log("sparse  ", JSON.stringify([3, , 1].sort()), JSON.stringify([3, undefined, 1].sort()));
console.log("typed   ", new Int32Array([3, 1, 2]).sort().join(","), new Int32Array([3, 1, 2]).sort((a, b) => b - a).join(","));
// A throwing comparator propagates rather than being swallowed.
try { [1, 2, 3].sort(() => { throw new Error("cmp"); }); } catch (e) { console.log("throws  ", e.message); }
