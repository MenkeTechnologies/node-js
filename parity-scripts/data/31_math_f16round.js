// Math.f16round: nearest IEEE 754 binary16, ties to even, from the double
// directly (no double rounding through float32), overflow to a signed infinity.
const xs = [1.337, 65504, 65519.99, 65520, -65520, 1e-8, 2.98e-8, 2.99e-8, 5.960464477539063e-8,
  8.940696716308594e-8, 1 / 3, 0.1, 6.1e-5, 6.103515625e-05, 6.097555160522461e-05,
  1.0009765625, 1.00048828125, 1.00146484375, 1.0004882812500002, 2049, 2051, 4097, 1e300, -1e-300,
  Number.MIN_VALUE, Number.MAX_VALUE, Math.PI, -Math.E, 3.4028235677973366e38];
for (const x of xs) console.log(x, Math.f16round(x));
console.log(Object.is(Math.f16round(-0), -0), Object.is(Math.f16round(-1e-10), -0), Math.f16round(NaN),
  Math.f16round(Infinity), Math.f16round(-Infinity), Math.f16round(), Math.f16round("0.5"), Math.f16round(1n === 1n));
console.log(Math.f16round.name, Math.f16round.length, Object.getOwnPropertyNames(Math).indexOf("f16round"));
console.log(Object.getOwnPropertyNames(JSON));
try { Math.f16round(1n); } catch (e) { console.log(e.name, e.message); }
