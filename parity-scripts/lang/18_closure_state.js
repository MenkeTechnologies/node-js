// Closures as state: counters, memoize, once, throttle-count.
function createCounter(start = 0) {
  let count = start;
  return {
    inc: () => ++count,
    dec: () => --count,
    value: () => count,
  };
}

const c = createCounter(10);
c.inc();
c.inc();
c.dec();
console.log("counter:", c.value());

// Memoization.
function memoize(fn) {
  const cache = new Map();
  return function (n) {
    if (cache.has(n)) return cache.get(n);
    const result = fn(n);
    cache.set(n, result);
    return result;
  };
}

let calls = 0;
const slowSquare = (n) => {
  calls++;
  return n * n;
};
const fastSquare = memoize(slowSquare);
console.log(fastSquare(4), fastSquare(4), fastSquare(5), fastSquare(4));
console.log("actual calls:", calls);

// once: run a fn a single time.
function once(fn) {
  let called = false;
  let result;
  return (...args) => {
    if (!called) {
      called = true;
      result = fn(...args);
    }
    return result;
  };
}
const init = once(() => "initialized");
console.log(init(), init(), init());

// Memoized fibonacci.
const fib = memoize(function f(n) {
  return n < 2 ? n : f(n - 1) + f(n - 2);
});
console.log("fib(20):", fib(20));
