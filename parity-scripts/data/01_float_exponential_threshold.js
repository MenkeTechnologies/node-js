// Float-to-string exponential-notation thresholds.
console.log((1e21).toString());        // 1e+21
console.log((1e20).toString());        // 100000000000000000000
console.log((9.999e20).toString());
console.log((1e-6).toString());        // 0.000001
console.log((1e-7).toString());        // 1e-7
console.log((1.5e-7).toString());
console.log((123456789012345680000).toString());
console.log((0.1 + 0.2).toString());   // 0.30000000000000004
console.log((-0).toString());          // 0
console.log((1 / -0).toString());      // -Infinity
console.log((123.456).toString());
console.log((1000000).toString());
console.log((0.0000001).toString());
console.log(String(2 ** 53));
console.log(String(2 ** 53 + 1));
