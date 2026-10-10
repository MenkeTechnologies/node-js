const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const v of [7, 'str', {}, true, 1n, Symbol('q')]) {
  console.log(P(() => +{ [Symbol.toPrimitive]: v }), P(() => new Date({ [Symbol.toPrimitive]: v })));
}
console.log(new Date({ [Symbol.toPrimitive]: () => 5 }).getTime(), new Date({ valueOf() { return 6; } }).getTime(), new Date({ toString() { return '2020-01-01'; } }).getTime(), new Date({ [Symbol.toPrimitive]: () => '2020-01-02' }).getTime());
console.log(P(() => Symbol.keyFor('x')), P(() => Symbol.keyFor(Symbol.for('k'))));
class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }
console.log(P(() => 2 instanceof Even), P(() => 3 instanceof Even), P(() => Function.prototype[Symbol.hasInstance].call(Even, 4)), P(() => Function.prototype[Symbol.hasInstance].call(Even, new Even())), P(() => Function.prototype[Symbol.hasInstance].call({}, {})));
console.log(P(() => ({}) instanceof (() => {})), P(() => ({}) instanceof (async function () {})), P(() => ({ m() {} }).m instanceof Object));
const re = /a/; re[Symbol.match] = true;
console.log(RegExp(re) === re, new RegExp(re) === re, RegExp({ [Symbol.match]: true, source: 'q', flags: 'g', constructor: RegExp }).flags, RegExp(/x/g, 'i').flags);
const m = new Map();
console.log(P(() => Object.assign(m, { [Symbol.toStringTag]: 'X' })), P(() => Object.assign(Object.freeze({}), { a: 1 })), P(() => { 'use strict'; m[Symbol.toStringTag] = 'Y'; }));
class S1 { static call(o) { return 'static call ' + o; } static apply() { return 'static apply'; } static bind() { return 'static bind'; } }
console.log(S1.call(1), S1.apply(), S1.bind());
