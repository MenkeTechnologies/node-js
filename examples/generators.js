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
