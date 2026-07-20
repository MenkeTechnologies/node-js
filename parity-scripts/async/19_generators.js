// Generators: lazy sequences, delegation (yield*), take-style consumption.
function* naturals() {
  let n = 1;
  while (true) yield n++;
}

function take(iter, count) {
  const out = [];
  for (const v of iter) {
    if (out.length >= count) break;
    out.push(v);
  }
  return out;
}

console.log('take5=' + take(naturals(), 5).join(','));

function* letters() {
  yield 'a';
  yield 'b';
}
function* combined() {
  yield* letters();
  yield* [1, 2];
  yield 'end';
}
console.log('delegate=' + [...combined()].join(','));

function* fibGen() {
  let [a, b] = [0, 1];
  while (true) {
    yield a;
    [a, b] = [b, a + b];
  }
}
console.log('fib=' + take(fibGen(), 8).join(','));

// two-way: value passed back into generator
function* echo() {
  const x = yield 'first';
  yield 'got:' + x;
}
const g = echo();
console.log('yield1=' + g.next().value);
console.log('yield2=' + g.next(42).value);
