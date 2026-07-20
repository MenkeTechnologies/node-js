// compose / pipe / curry higher-order function pipelines.
const compose = (...fns) => (x) => fns.reduceRight((acc, f) => f(acc), x);
const pipe = (...fns) => (x) => fns.reduce((acc, f) => f(acc), x);

const inc = (n) => n + 1;
const dbl = (n) => n * 2;
const sq = (n) => n * n;

console.log('compose=' + compose(inc, dbl, sq)(3)); // sq->dbl->inc = 19
console.log('pipe=' + pipe(inc, dbl, sq)(3));        // inc->dbl->sq = 64

const curry = (fn) => {
  const arity = fn.length;
  const collect = (args) =>
    args.length >= arity ? fn(...args) : (...more) => collect([...args, ...more]);
  return (...args) => collect(args);
};

const add3 = curry((a, b, c) => a + b + c);
console.log('curry-full=' + add3(1, 2, 3));
console.log('curry-partial=' + add3(1)(2)(3));
console.log('curry-mixed=' + add3(1, 2)(3));

const pipeline = pipe(
  (xs) => xs.filter((n) => n % 2 === 0),
  (xs) => xs.map(sq),
  (xs) => xs.reduce((a, b) => a + b, 0),
);
console.log('pipeline=' + pipeline([1, 2, 3, 4, 5, 6]));
