// Generators: function*, yield, yield*, .next() sequencing, spread iteration.
function* range(start, end) {
  for (let i = start; i < end; i++) {
    yield i;
  }
}

function* fibonacci(n) {
  let a = 0, b = 1;
  for (let i = 0; i < n; i++) {
    yield a;
    [a, b] = [b, a + b];
  }
}

function* concat(...iterables) {
  for (const it of iterables) {
    yield* it;
  }
}

console.log("range", [...range(2, 7)]);
console.log("fib", Array.from(fibonacci(6)));
console.log("concat", [...concat([1, 2], range(10, 13), [99])]);

const g = range(0, 100);
console.log("manual", g.next().value, g.next().value, g.next().value);

let total = 0;
for (const v of fibonacci(8)) {
  total += v;
}
console.log("sum of fib(8)", total);

console.log("max via spread", Math.max(...range(3, 9)));

function* echo() {
  while (true) {
    const got = yield;
    if (got === undefined) return;
    console.log("echo:", got);
  }
}
const e = echo();
e.next();
e.next("a");
e.next("b");

// A generator or async function is not an ordinary function: it has its own
// intrinsic constructor and prototype. All three reported plain `Function`, so
// `g.constructor.name` was `Function` where node says `GeneratorFunction`, and
// `Object.getPrototypeOf(g) === Function.prototype` was wrongly true.
function* syncGen() { yield 1; }
async function asyncFn() {}
async function* asyncGen() { yield 1; yield 2; }
function plain() {}
const kindOf = (f) => Object.getPrototypeOf(f).constructor.name;
console.log("kinds   ", kindOf(syncGen), kindOf(asyncFn), kindOf(asyncGen), kindOf(plain));
console.log("ctor    ", syncGen.constructor.name, asyncFn.constructor.name,
  asyncGen.constructor.name, plain.constructor === Function);
// Only an ordinary function inherits from `Function.prototype`.
console.log("chain   ", Object.getPrototypeOf(plain) === Function.prototype,
  Object.getPrototypeOf(syncGen) === Function.prototype,
  typeof Object.getPrototypeOf(syncGen));
// Each intrinsic prototype carries the tag that names it.
const GenFn = Object.getPrototypeOf(syncGen).constructor;
const AsyncFn = Object.getPrototypeOf(asyncFn).constructor;
const AsyncGenFn = Object.getPrototypeOf(asyncGen).constructor;
console.log("tags    ", GenFn.prototype[Symbol.toStringTag],
  AsyncFn.prototype[Symbol.toStringTag], AsyncGenFn.prototype[Symbol.toStringTag]);
console.log("protos  ", typeof GenFn.prototype, typeof AsyncFn.prototype,
  GenFn.prototype !== AsyncFn.prototype);
// The brand each function value reports is unchanged.
console.log("brands  ", Object.prototype.toString.call(syncGen),
  Object.prototype.toString.call(asyncFn), Object.prototype.toString.call(asyncGen),
  Object.prototype.toString.call(plain));

// A generator IS its own iterator, so it answers for the matching symbol — the
// async one for `Symbol.asyncIterator`, the sync one for `Symbol.iterator`.
// Neither was advertised: `asyncGen()[Symbol.asyncIterator]` read `undefined`
// even though `for await` over it already worked through another path.
const ai = asyncGen();
console.log("asyncSym", typeof ai[Symbol.asyncIterator], ai[Symbol.asyncIterator]() === ai);
const si = syncGen();
console.log("syncSym ", typeof si[Symbol.iterator], si[Symbol.iterator]() === si);
console.log("instTag ", Object.prototype.toString.call(syncGen()),
  Object.prototype.toString.call(asyncGen()));
(async () => {
  const seen = [];
  for await (const x of asyncGen()) { seen.push(x); }
  console.log("forawait", seen.join(","));
})();
