// Number formatting, parsing, and the integer/float boundaries.
console.log(0.1 + 0.2, (0.1 + 0.2).toFixed(2), (1234.5678).toFixed(2));
console.log((1234.5678).toPrecision(6), (255).toString(16), (255).toString(2));
console.log(parseInt("42px"), parseInt("1f", 16), parseFloat("3.14abc"));
console.log(Number("42"), Number(""), Number("abc"), Number(null), Number(true));
console.log(Number.isInteger(5), Number.isInteger(5.5), Number.isFinite(1 / 0));
console.log(Number.MAX_SAFE_INTEGER, Number.EPSILON > 0);
console.log(Math.max(1, 9, 3), Math.min(1, 9, 3), Math.abs(-7));
console.log(Math.floor(-1.5), Math.ceil(-1.5), Math.round(-1.5), Math.trunc(-1.5));
console.log(Math.sign(-3), Math.sign(0), Math.pow(2, 10), 2 ** 10);
console.log(7 / 2, 7 % 3, -7 % 3, Math.hypot(3, 4));
console.log(1 / 0, -1 / 0, 0 / 0, Object.is(NaN, NaN), Object.is(0, -0));
