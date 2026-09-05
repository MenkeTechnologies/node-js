// Borrowing a method off a builtin prototype, and what a nullish base reports.
//
// `X.prototype.m.call(v)` is how library code calls a builtin method on a value
// that does not inherit it. It worked for `Array`/`Object`/`Date`/`RegExp`/
// `Map`/`Set` and threw for `String`/`Number`/`Boolean`, whose prototype
// objects carried only the three conversion methods.

console.log('string  ', String.prototype.trim.call('  x  '), String.prototype.charAt.call('ab', 1));
console.log('string2 ', String.prototype.toUpperCase.call('ab'), String.prototype.slice.call('abcd', 1, 3));
console.log('number  ', Number.prototype.toFixed.call(1.005, 2), Number.prototype.toString.call(255, 16));
console.log('boolean ', Boolean.prototype.valueOf.call(true), Boolean.prototype.toString.call(false));
console.log('typeof  ', typeof String.prototype.trim, typeof Number.prototype.toFixed, typeof Boolean.prototype.valueOf);

// The already-working borrows, so a regression in either direction shows here.
console.log('generic ', Array.prototype.slice.call({ 0: 'a', 1: 'b', length: 2 }));
console.log('own     ', Object.prototype.hasOwnProperty.call({ a: 1 }, 'a'));
console.log('date    ', Date.prototype.getTime.call(new Date(7)));
console.log('regexp  ', RegExp.prototype.test.call(/a/, 'a'), Map.prototype.get.call(new Map([[1, 2]]), 1));

// A method call on a nullish base fails at the property READ, and the message
// names the base — not the callee.
const msg = (f) => { try { f(); return 'no throw'; } catch (e) { return e.constructor.name + ': ' + e.message; } };
console.log('undef   ', msg(() => undefined.foo()));
console.log('null    ', msg(() => null.foo()));
console.log('chain   ', msg(() => { const o = {}; return o.a.b(); }));
console.log('read    ', msg(() => undefined.foo));
console.log('index   ', msg(() => { const o = null; return o[0]; }));
// A present base with an absent method still reports the CALL.
console.log('absent  ', msg(() => ({}).nope()));
console.log('absent2 ', msg(() => [].nope()));
// Optional chaining short-circuits instead of throwing.
console.log('optional', undefined?.foo, null?.foo?.(), ({})?.nope?.());
