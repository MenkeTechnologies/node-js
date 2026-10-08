// Date.prototype[Symbol.toPrimitive] (21.4.4.45) is generic over any object:
// it validates the hint, then runs OrdinaryToPrimitive through [[Get]], so a
// Proxy's get trap sees each method read.
const tp = Date.prototype[Symbol.toPrimitive];
const show = (f) => { try { console.log(f()); } catch (e) { console.log(e.constructor.name + ': ' + e.message); } };
for (const r of [1, 's', null, undefined]) show(() => tp.call(r, 'number'));
for (const h of ['x', undefined, 1, { toString() { return 'number'; } }]) show(() => tp.call(new Date(0), h));
show(() => tp.call({ valueOf() { return 5; }, toString() { return 't'; } }, 'default'));
show(() => tp.call({ valueOf() { return 5; } }, 'number'));
show(() => tp.call([1, 2], 'string'));
show(() => tp.call({ valueOf() { return {}; }, toString() { return {}; } }, 'number'));
const d = new Date(0);
d.toString = () => 'own';
console.log(`${d}`, d + 1, tp.call(d, 'number'));
const reads = [];
const dp = new Proxy(new Date(0), {
  get(t, k) { reads.push(String(k)); const v = Reflect.get(t, k, t); return typeof v === 'function' ? v.bind(t) : v; },
});
show(() => tp.call(dp, 'number'));
show(() => tp.call(dp, 'string'));
console.log(reads.join(' '));
console.log(tp.length, tp.name);
