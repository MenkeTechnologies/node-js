// Set construction, dedup, SameValueZero (NaN equal to NaN), operations.
const s = new Set([1, 2, 2, 3, 3, 3]);
s.add(4);
s.add(NaN);
s.add(NaN); // SameValueZero => single NaN
s.delete(2);

console.log('size=' + s.size);
console.log('has-3=' + s.has(3));
console.log('has-2=' + s.has(2));
console.log('has-NaN=' + s.has(NaN));

const seen = [];
s.forEach((v) => seen.push(String(v)));
console.log('forEach=' + seen.join(','));

console.log('values=' + [...s.values()].map(String).join(','));
console.log('spread=' + [...s].map(String).join(','));

// Dedup an array via Set.
const dedup = [...new Set(['x', 'y', 'x', 'z', 'y'])];
console.log('dedup=' + dedup.join(','));
