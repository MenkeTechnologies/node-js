// NaN / Infinity handling.
console.log(NaN === NaN);               // false
console.log(Number.isNaN(NaN), isNaN("x"));
console.log(Object.is(NaN, NaN));       // true
console.log(1 / 0, -1 / 0, 0 / 0);
console.log(Infinity + 1, Infinity - Infinity);
console.log(Infinity * 0);              // NaN
console.log(Math.max(), Math.min());    // -Infinity, Infinity
console.log(Number.MAX_VALUE * 2);      // Infinity
console.log(Number.isFinite(Infinity), Number.isFinite(42));
console.log(Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER);
console.log(Number.EPSILON);
console.log(Math.sqrt(-1));             // NaN
console.log(NaN + 1, NaN * 0);
console.log([NaN].includes(NaN));       // true
console.log([NaN].indexOf(NaN));        // -1
console.log(parseFloat("1e999"));       // Infinity
console.log(JSON.stringify([NaN, Infinity, -Infinity])); // nulls
