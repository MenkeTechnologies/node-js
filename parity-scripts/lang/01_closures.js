// Closures and higher-order functions.
function makeAdder(x) {
  return function (y) {
    return x + y;
  };
}

function compose(...fns) {
  return (arg) => fns.reduceRight((acc, fn) => fn(acc), arg);
}

const add5 = makeAdder(5);
const add10 = makeAdder(10);
console.log(add5(3), add10(3));

const double = (n) => n * 2;
const inc = (n) => n + 1;
const square = (n) => n * n;

const pipeline = compose(double, inc, square);
console.log(pipeline(3)); // square(3)=9 -> inc=10 -> double=20

const nums = [1, 2, 3, 4, 5, 6];
const result = nums
  .filter((n) => n % 2 === 0)
  .map(double)
  .reduce((a, b) => a + b, 0);
console.log(result);

// Closure capturing loop variable with let.
const fns = [];
for (let i = 0; i < 3; i++) {
  fns.push(() => i);
}
console.log(fns.map((f) => f()).join(","));
