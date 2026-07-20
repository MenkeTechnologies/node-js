// Memoization with a Map cache; deterministic call-count tracking.
function memoize(fn) {
  const cache = new Map();
  let misses = 0;
  const memo = (n) => {
    if (cache.has(n)) return cache.get(n);
    misses += 1;
    const result = fn(n);
    cache.set(n, result);
    return result;
  };
  memo.misses = () => misses;
  return memo;
}

const fib = memoize(function f(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
});

console.log('fib10=' + fib(10));
console.log('fib15=' + fib(15));
console.log('fib10-again=' + fib(10));
// With memoized recursion, unique n values 0..15 are computed once each.
console.log('misses=' + fib.misses());
