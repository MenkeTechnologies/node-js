// Recursion, closures, and arrow functions.
function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}
const seq = [];
for (let i = 0; i < 15; i++) seq.push(fib(i));
console.log("fib:", seq.join(", "));

const makeCounter = () => {
  let count = 0;
  return () => ++count;
};
const c = makeCounter();
console.log(c(), c(), c());
