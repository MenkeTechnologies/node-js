// Map construction from iterable, get/set/has/delete/size, insertion order.
const m = new Map([
  ['a', 1],
  ['b', 2],
  ['c', 3],
]);

m.set('d', 4);
m.set('a', 10); // overwrite keeps original position
m.delete('b');

console.log('size=' + m.size);
console.log('has-c=' + m.has('c'));
console.log('has-b=' + m.has('b'));
console.log('get-a=' + m.get('a'));

const entries = [];
m.forEach((v, k) => entries.push(k + ':' + v));
console.log('forEach=' + entries.join(','));

console.log('keys=' + [...m.keys()].join(','));
console.log('values=' + [...m.values()].join(','));
console.log('spread=' + JSON.stringify([...m]));
