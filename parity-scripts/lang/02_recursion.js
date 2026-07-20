// Classic recursion: fibonacci, factorial, small Ackermann.
function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

function factorial(n) {
  return n <= 1 ? 1 : n * factorial(n - 1);
}

function ackermann(m, n) {
  if (m === 0) return n + 1;
  if (n === 0) return ackermann(m - 1, 1);
  return ackermann(m - 1, ackermann(m, n - 1));
}

function gcd(a, b) {
  return b === 0 ? a : gcd(b, a % b);
}

const fibs = [];
for (let i = 0; i < 12; i++) fibs.push(fib(i));
console.log("fib:", fibs.join(" "));

const facts = [];
for (let i = 1; i <= 10; i++) facts.push(factorial(i));
console.log("fact:", facts.join(" "));

console.log("ackermann(2,3):", ackermann(2, 3));
console.log("ackermann(3,3):", ackermann(3, 3));
console.log("gcd(48,36):", gcd(48, 36));
