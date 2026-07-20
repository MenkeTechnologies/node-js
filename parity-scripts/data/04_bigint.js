// BigInt arithmetic, factorials, asIntN, mixing TypeError caught.
function bigFactorial(n) {
  let acc = 1n;
  for (let i = 2n; i <= n; i++) acc *= i;
  return acc;
}
console.log(bigFactorial(20n).toString());
console.log(bigFactorial(50n).toString());
console.log(bigFactorial(100n).toString());

console.log((2n ** 64n).toString());
console.log((2n ** 100n).toString());
console.log((-7n % 3n).toString());
console.log((10n ** 30n / 7n).toString());

console.log(BigInt.asIntN(8, 255n).toString());   // -1
console.log(BigInt.asIntN(16, 40000n).toString());
console.log(BigInt.asUintN(8, -1n).toString());   // 255
console.log(BigInt.asUintN(4, 17n).toString());   // 1

try {
  // mixing BigInt and Number throws TypeError
  console.log(1n + 1);
} catch (e) {
  console.log(e.constructor.name + ": mixed");
}

console.log((9007199254740993n).toString()); // exceeds Number precision
console.log(typeof 1n);
