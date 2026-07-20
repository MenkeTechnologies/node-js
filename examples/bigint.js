// BigInt: arbitrary-precision integers (typeof === "bigint").

// Literals and formatting.
console.log(10n, -3n, typeof 10n);
console.log(String(255n), (255n).toString(16), (255n).toString(2));

// Arithmetic: + - * / % ** (division truncates toward zero).
console.log(7n / 2n, -7n / 2n, 7n % 3n, -7n % 3n, 2n ** 10n);

// A big factorial stays exact where a Number (f64) would lose precision.
let fact = 1n;
for (let i = 1n; i <= 25n; i++) fact *= i;
console.log(fact);

// Comparisons work across BigInt and Number; `==` coerces, `===` does not.
console.log(10n < 20, 10n == 10, 10n === 10, 9007199254740993n === 9007199254740993n);

// Bitwise on BigInt is arbitrary width.
console.log(5n & 3n, 5n | 2n, 5n ^ 1n, ~5n, 1n << 8n);

// Mixing a BigInt with a Number in arithmetic is a hard TypeError.
try {
  console.log(1n + 1);
} catch (e) {
  console.log(e.constructor.name + ": " + e.message);
}

// The BigInt(...) constructor.
console.log(BigInt(42), BigInt("0xff"), BigInt(true));
