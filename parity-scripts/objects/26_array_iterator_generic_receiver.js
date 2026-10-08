// The built-in array iterator over a Proxy or an array-like re-reads `length`
// through [[Get]] at every step and never calls `has` (23.1.5.1); a for-of
// body runs between steps, and destructuring takes only as many steps as it binds.
const log = [];
const p = new Proxy([1, 2, 3], {
  get(t, k, r) { log.push('get:' + String(k)); return Reflect.get(t, k, r); },
  has(t, k) { log.push('has:' + String(k)); return k in t; },
});
for (const x of p) {}
console.log(log.join(' '));
log.length = 0;
console.log([...p].length, log.join(' '));
log.length = 0;
const pit = Array.prototype.values.call(p);
pit.next();
console.log(log.join(' '));

const al = { length: 2, 0: 'a', 1: 'b' };
const it = Array.prototype.values.call(al);
console.log(it.next());
al[2] = 'c';
al.length = 3;
console.log([...it]);
console.log([...Array.prototype.entries.call('xy')], [...Array.prototype.keys.call({ length: '2' })]);
console.log(Object.prototype.toString.call(Array.prototype.values.call({})));

const arr = [1, 2];
for (const x of new Proxy(arr, {})) {
  if (arr.length < 4) arr.push(x * 10);
  console.log(x);
}
try {
  [...Array.prototype.values.call({ get length() { throw new Error('boom'); } })];
} catch (e) {
  console.log(e.message);
}
const seen = [];
const q = new Proxy([7, 8, 9], { get(t, k, r) { seen.push(String(k)); return Reflect.get(t, k, r); } });
const [a, b] = q;
console.log(a, b, seen.join());
const [...rest] = q;
console.log(rest, Array.from(q), Array.from(Array.prototype.values.call({ length: 1, 0: 5 })));
console.log([...Array.prototype.values.call(new Uint8Array([3, 4]))]);
const it2 = [].values.call({ length: 1, 0: 1 });
console.log(it2.next(), it2.next(), it2.next());
console.log(Array.prototype.keys.call({ length: Infinity }).next());
