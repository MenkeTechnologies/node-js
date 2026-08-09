// Symbol.iterator / Symbol.asyncIterator protocols and manual iterator driving.
class Range {
  constructor(n) { this.n = n; }
  *[Symbol.iterator]() { for (let i = 0; i < this.n; i++) yield i; }
}
console.log(JSON.stringify([...new Range(4)]));
console.log(JSON.stringify(Array.from(new Range(3), (x) => x * 2)));
const [a, b] = new Range(5);
console.log(a, b);
for (const v of new Range(2)) console.log("of", v);

// A hand-rolled (non-generator) iterator object.
const manual = {
  [Symbol.iterator]() {
    let i = 0;
    return { next: () => (i < 3 ? { value: i++, done: false } : { value: undefined, done: true }) };
  },
};
console.log(JSON.stringify([...manual]));

// Driving a generator's iterator by hand, including the return value.
function* g() { yield 1; yield 2; return 99; }
const it = g();
console.log(JSON.stringify(it.next()), JSON.stringify(it.next()), JSON.stringify(it.next()), JSON.stringify(it.next()));
console.log(JSON.stringify([...g()]));

// Delegation and built-in iterables.
function* outer() { yield "a"; yield* g(); yield "z"; }
console.log(JSON.stringify([...outer()]));
console.log(typeof [][Symbol.iterator], typeof ""[Symbol.iterator], typeof new Set()[Symbol.iterator]);
console.log(JSON.stringify([..."héllo"]));
console.log(JSON.stringify([...new Map([["k", 1]])]));

async function* ag() { yield 1; yield 2; }
(async () => {
  const out = [];
  for await (const v of ag()) out.push(v);
  console.log("async", JSON.stringify(out));
  const arr = [Promise.resolve("p1"), "p2"];
  const out2 = [];
  for await (const v of arr) out2.push(v);
  console.log("forawait-array", JSON.stringify(out2));
})();
