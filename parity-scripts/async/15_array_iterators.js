// Array entries/keys/values iterators + Array.from with map fn + of.
const arr = ['x', 'y', 'z'];

const ent = [];
for (const [i, v] of arr.entries()) ent.push(i + '=' + v);
console.log('entries=' + ent.join(','));

console.log('keys=' + [...arr.keys()].join(','));
console.log('values=' + [...arr.values()].join(','));

console.log('from-range=' + Array.from({ length: 5 }, (_, i) => i * i).join(','));
console.log('from-set=' + Array.from(new Set([1, 1, 2, 3])).join(','));
console.log('from-string=' + Array.from('abc').join('-'));
console.log('of=' + Array.of(7, 8, 9).join(','));

// join with entries destructuring on a Map
const m = new Map([['k1', 1], ['k2', 2]]);
console.log('map-entries=' + [...m.entries()].map(([k, v]) => k + v).join(','));
