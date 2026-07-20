// toFixed, toPrecision, toString(radix) formatting.
const n = 3.14159265358979;
for (let d = 0; d <= 6; d++) console.log(n.toFixed(d));

for (let p = 1; p <= 8; p++) console.log(n.toPrecision(p));

console.log((255).toString(16));
console.log((255).toString(2));
console.log((255).toString(8));
console.log((3735928559).toString(16));
console.log((1000000).toString(36));
console.log((-42).toString(2));
console.log((0.5).toString(2));
console.log((255.75).toString(16));

console.log((1234.5678).toFixed(2));
console.log((0).toFixed(3));
console.log((1e-7).toFixed(10));
console.log((12345.6789).toPrecision(4));
console.log((0.00012345).toPrecision(3));
console.log((100).toExponential(2));
console.log((0.000123).toExponential(4));
