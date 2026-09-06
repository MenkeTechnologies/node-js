// What a builtin IS, rather than what it returns: the `name` and `length` every
// builtin function carries, its native-code source form, the descriptor of a
// namespace member, the own-name lists, and the branded TypeError a prototype
// method throws when its receiver has no such internal slot.

for (const f of [Math.max, Object.keys, parseInt, Array.prototype.slice, Set.prototype.union]) {
  console.log('fn        ', f.name, f.length, typeof f.call, typeof f.bind, String(f));
}
console.log('bound     ', [].slice.name, [].slice.length, new Map().get.name, 'x'.padStart.name);
console.log('inspect   ', Uint8Array.prototype.set, [].slice, Math.max);
console.log('ctor      ', Array.name, Array.length, Map.length, typeof Set.prototype);

// A namespace object is not a function.
console.log('namespace ', typeof Math, Math instanceof Function, Math instanceof Object);
console.log('brand     ', Object.prototype.toString.call(Math), Object.prototype.toString.call(Set.prototype));
console.log('enumerate ', Object.keys(Math).length, JSON.stringify(Math), Math.name);
console.log('names     ', Object.getOwnPropertyNames(Number).join(','));
console.log('fnNames   ', Object.getOwnPropertyNames(Math.max).join(','));

// Descriptors: a constant is frozen, a function's own name/length is read-only
// but configurable, and an ordinary method is writable.
for (const [o, k, label] of [
  [Math, 'PI', 'Math.PI'],
  [Math, 'floor', 'Math.floor'],
  [Number, 'MAX_SAFE_INTEGER', 'Number.MAX'],
  [Number, 'prototype', 'Number.proto'],
  [Math.max, 'name', 'max.name'],
  [Array.prototype, 'slice', 'A.p.slice'],
  [globalThis, 'Math', 'global.Math'],
  [globalThis, 'undefined', 'global.undef'],
  [globalThis, 'structuredClone', 'global.sClone'],
]) {
  const d = Object.getOwnPropertyDescriptor(o, k);
  console.log('descr     ', label.padEnd(13), d ? `w=${d.writable} e=${d.enumerable} c=${d.configurable}` : 'undefined');
}

// The brand check names the method and renders the receiver the way V8's
// side-effect-free stringifier does: a primitive by value, an object that still
// inherits Object.prototype.toString as `#<Ctor>`, and anything with its own
// toString by its builtin brand.
const receivers = [[], {}, 5, 'str', null, undefined, new Map(), new Set(), new Date(0), Object.create(null), new (class A {})(), true, 9n, new Uint8Array(1), Promise.resolve(1), new WeakMap()];
const methods = [
  ['Set.prototype.union', (r) => Set.prototype.union.call(r, new Set())],
  ['Map.prototype.get', (r) => Map.prototype.get.call(r, 1)],
  ['Promise.prototype.then', (r) => Promise.prototype.then.call(r, () => {})],
  ['Date.prototype.getTime', (r) => Date.prototype.getTime.call(r)],
  ['Date.prototype.toISOString', (r) => Date.prototype.toISOString.call(r)],
  ['Uint8Array.prototype.slice', (r) => Uint8Array.prototype.slice.call(r)],
  ['Uint8Array.prototype.fill', (r) => Uint8Array.prototype.fill.call(r)],
];
for (const [label, call] of methods) {
  for (const r of receivers) {
    let out;
    try {
      call(r);
      out = 'ok';
    } catch (e) {
      out = `${e.constructor.name}: ${e.message}`;
    }
    console.log('brandcheck', label.padEnd(28), out);
  }
}
