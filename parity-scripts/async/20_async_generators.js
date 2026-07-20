// Async generators + for-await-of over resolved promises (deterministic order).
async function* asyncCounter(limit) {
  for (let i = 1; i <= limit; i++) {
    yield await Promise.resolve(i * i);
  }
}

async function main() {
  const collected = [];
  for await (const v of asyncCounter(5)) {
    collected.push(v);
  }
  console.log('for-await=' + collected.join(','));

  // for-await over an array of promises resolves in array order.
  const promises = [Promise.resolve('a'), Promise.resolve('b'), Promise.resolve('c')];
  const seq = [];
  for await (const v of promises) seq.push(v);
  console.log('promise-array=' + seq.join(','));

  // async delegation via yield*
  async function* wrapped() {
    yield* asyncCounter(3);
    yield await Promise.resolve(100);
  }
  const w = [];
  for await (const v of wrapped()) w.push(v);
  console.log('wrapped=' + w.join(','));
}
main();
