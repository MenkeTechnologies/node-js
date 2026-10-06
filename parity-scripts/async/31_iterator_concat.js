// Iterator.concat: up-front validation, lazy opening, value pass-through,
// return() closing the iterator in flight (also for flatMap), and the
// statics of an ES constructor being non-enumerable.
const log = [];
function* g(n) { try { for (let i = 0; i < n; i++) { log.push("y" + i); yield i; } } finally { log.push("fin" + n); } }
const it = Iterator.concat(g(2), [10, 11], new Set(["s"]), "ab".split(""));
console.log(Object.prototype.toString.call(it), typeof it.next, it.toArray(), log.splice(0));
const lazy = Iterator.concat(g(3), g(4)); console.log(log.length, lazy.next(), log.splice(0));
console.log(lazy.return(), log.splice(0), lazy.next());
console.log(Iterator.concat().next(), Iterator.concat.length, Iterator.concat.name);
const fm = [1, 2].values().flatMap((x) => g(x + 1)); fm.next(); console.log(fm.return(), log.splice(0));
for (const bad of [1, "str", null, undefined, {}, { [Symbol.iterator]: 1 }]) {
  try { Iterator.concat([1], bad); } catch (e) { console.log(e.name, e.message); }
}
let opened = 0;
const lazyOpen = { [Symbol.iterator]() { opened++; return [7][Symbol.iterator](); } };
const c = Iterator.concat(lazyOpen, lazyOpen); console.log(opened, c.next(), opened, c.next(), opened, [...c], opened);
try { Iterator.concat({ [Symbol.iterator]() { return 5; } }).next(); } catch (e) { console.log(e.name, e.message); }
const m = Iterator.concat([1, 2], [3]).map((x) => x * 2).filter((x) => x > 2); console.log(m.toArray());
const passthrough = Iterator.concat({ [Symbol.iterator]() { return { next() { return { value: 1, done: false, extra: 1 }; } }; } });
console.log(passthrough.next());
console.log(Object.getOwnPropertyNames(Iterator), Object.keys(Iterator), Object.getOwnPropertyDescriptor(Iterator, "concat"));
for (const c of [Promise, Buffer, URL]) console.log(c.name, Object.getOwnPropertyNames(c).slice(0, 3), Object.getOwnPropertyNames(Promise).slice(3));
