// Destructuring, defaults, rest/spread, and optional chaining.
const { a, b: renamed, missing = "dflt", ...others } = { a: 1, b: 2, c: 3, d: 4 };
console.log(a, renamed, missing, others);
const [x, , z = 99, ...tail] = [10, 20, undefined, 40, 50];
console.log(x, z, tail);
const { deep: { inner } } = { deep: { inner: "found" } };
console.log(inner);
let p = 1, q = 2;
[p, q] = [q, p];
console.log(p, q);
const key = "dyn";
const { [key]: viaComputed } = { dyn: "computed" };
console.log(viaComputed);
function f({ n = 5, m } = {}, ...rest) { return [n, m, rest]; }
console.log(f(), f({ n: 1, m: 2 }, 3, 4));
const obj = { nested: { arr: [1, 2] } };
console.log(obj?.nested?.arr?.[1], obj?.nope?.deep, obj.nope?.().x);
console.log(null ?? "fallback", 0 ?? "no", "" || "empty-is-falsy", 0 || "zero");
let lv = null; lv ??= "assigned"; console.log(lv);
let lv2 = 5; lv2 ||= 9; lv2 &&= 7; console.log(lv2);
console.log({ ...{ s: 1 }, ...{ t: 2 } }, [...[1, 2], ...[3]]);

// An array pattern pulls exactly as many values as it names, then closes the
// iterator (8.6.2 IteratorBindingInitialization). Draining instead is not just
// slower — it is observable as the `next()` count and as whether `return()`
// ran, and it never terminates against an unbounded source.
// The source is unbounded, but throws once pulled past a small budget rather
// than looping forever: a regression here should fail the record in a moment,
// not sit until the harness' timeout fires.
function counting(budget = 4) {
  let pulls = 0, closed = 0;
  const src = {
    [Symbol.iterator]() {
      let i = 0;
      return {
        next: () => {
          if (++pulls > budget) throw new Error("iterator drained past the pattern");
          return { value: i++, done: false };
        },
        return() { closed++; return { done: true }; },
      };
    },
  };
  return { src, stats: () => [pulls, closed] };
}
const one = counting(); const [only] = one.src;
console.log("take1   ", only, one.stats().join(","));
const two = counting(); const [, second] = two.src;
console.log("hole    ", second, two.stats().join(","));
// A `...rest` element does consume the remainder, so a finite source is drained
// and there is nothing left to close.
const fin = { *[Symbol.iterator]() { yield 1; yield 2; yield 3; } };
const [restHead, ...restTail] = fin;
console.log("rest    ", restHead, restTail.join(","));
// Closing a generator early resumes it at the yield, so `finally` runs.
function* withCleanup() { try { yield "a"; yield "b"; } finally { console.log("cleanup "); } }
const [firstOnly] = withCleanup();
console.log("gen     ", firstOnly);
