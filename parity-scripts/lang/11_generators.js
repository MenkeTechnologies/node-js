// Generators: yield, yield*, delegation, infinite + take.
function* range(start, end, step = 1) {
  for (let i = start; i < end; i += step) yield i;
}

function* fibGen() {
  let [a, b] = [0, 1];
  while (true) {
    yield a;
    [a, b] = [b, a + b];
  }
}

function take(gen, n) {
  const out = [];
  for (const v of gen) {
    if (out.length >= n) break;
    out.push(v);
  }
  return out;
}

console.log("range:", [...range(0, 10, 2)].join(","));
console.log("fib:", take(fibGen(), 10).join(","));

// Delegation with yield*.
function* letters() {
  yield "a";
  yield "b";
}
function* combined() {
  yield* letters();
  yield* range(1, 4);
  yield "z";
}
console.log("combined:", [...combined()].join(","));

// Generator returning a value.
function* counter() {
  yield 1;
  yield 2;
  return "done";
}
const g = counter();
console.log(g.next().value, g.next().value, g.next().value, g.next().done);
