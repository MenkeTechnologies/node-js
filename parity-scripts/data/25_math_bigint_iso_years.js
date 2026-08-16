// Math coerces with ToNumber, and ToNumber of a BigInt throws.
for (const e of ['Math.abs(1n)', 'Math.max(1n,2)', 'Math.min(1n)', 'Math.hypot(1n)',
                 'Math.pow(2n,2)', 'Math.atan2(1,2n)', 'Math.imul(1n,2)', 'Math.clz32(1n)']) {
  try { eval(e); console.log('NO-THROW ' + e); } catch (err) { console.log(err.constructor.name + ': ' + err.message); }
}
// Math.random never reads an argument.
console.log(typeof Math.random(1n));
console.log(Math.max(1, 2, 3), Math.hypot(3, 4), Math.min());
// ISO years outside 0..9999 use the signed six-digit expanded form.
console.log(new Date(8.64e15).toISOString());
console.log(new Date(-8.64e15).toISOString());
console.log(new Date(Date.UTC(-1, 0, 1)).toISOString());
console.log(new Date(Date.UTC(10000, 0, 1)).toISOString());
console.log(new Date(Date.UTC(9999, 11, 31)).toISOString());
console.log(new Date(Date.UTC(0, 0, 1)).toISOString());
console.log(new Date(0).toISOString(), JSON.stringify(new Date(-62198755200000)));
