// A `<C>.prototype` is an ORDINARY object. It reports its constructor's brand
// only where the specification really gives it one: the ES5 legacy prototypes
// that carry an internal slot (Array, Function, String, Number, Boolean) and the
// ones that carry an own `Symbol.toStringTag`. Date, RegExp and every Error
// prototype are plain objects and report `[object Object]`.
const brand = Object.prototype.toString;
const names = [
  'Object', 'Array', 'Function', 'Date', 'RegExp', 'Error', 'TypeError',
  'Map', 'Set', 'WeakMap', 'WeakSet', 'Promise', 'Symbol', 'BigInt',
  'Number', 'String', 'Boolean', 'ArrayBuffer', 'DataView', 'Uint8Array',
  'WeakRef', 'FinalizationRegistry',
];
for (const n of names) {
  const proto = globalThis[n].prototype;
  console.log(n.padEnd(21), brand.call(proto), String(proto[Symbol.toStringTag]));
}

// The brand a prototype reports is the brand its own failure messages quote.
try {
  Date.prototype.toString.call({});
} catch (e) {
  console.log(e.message);
}
try {
  String(Date.prototype);
} catch (e) {
  console.log(e.message);
}

// A prototype built as a real object still advertises its symbol-keyed methods.
console.log(typeof String.prototype[Symbol.iterator], String.prototype[Symbol.iterator].name);
console.log([...String.prototype[Symbol.iterator].call('ab')].join('|'));
console.log(typeof Array.prototype[Symbol.iterator], Array.prototype[Symbol.iterator].name);

// `Object.prototype` is the chain root: its own `[[Prototype]]` is null, so
// `util.inspect` tags it the way it tags any other null-prototype object.
console.log(Object.getPrototypeOf(Object.prototype));
console.log(Object.prototype);
console.log(Object.getOwnPropertyDescriptor(Object, 'prototype'));
console.log(Object.create(null));
