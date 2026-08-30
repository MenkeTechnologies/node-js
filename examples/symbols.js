// Well-known symbols, the protocols they drive, and the Proxy traps — none of
// which the corpus covered. The trap LOG is the point of the proxy section: it
// pins the observable sequence, which is how a stray `get` behind `Object.keys`
// shows up at all.
const it = { *[Symbol.iterator]() { yield 1; yield 2; yield 3; } };
console.log('iterator', [...it].join(','), Array.from(it).join(','));

const prim = {
  [Symbol.toPrimitive](hint) { return hint === 'number' ? 42 : 'str:' + hint; },
};
console.log('toPrim  ', +prim, `${prim}`, prim + '');

class Even { static [Symbol.hasInstance](n) { return typeof n === 'number' && n % 2 === 0; } }
console.log('hasInst ', 4 instanceof Even, 5 instanceof Even);

const tagged = { get [Symbol.toStringTag]() { return 'Custom'; } };
console.log('tag     ', Object.prototype.toString.call(tagged));

const s1 = Symbol('d'), s2 = Symbol.for('k');
console.log('symbols ', s1.toString(), s1.description, Symbol.keyFor(s2), s2 === Symbol.for('k'));

const withSym = { [s1]: 'v', plain: 1 };
console.log('keys    ', Object.keys(withSym).join(','), Object.getOwnPropertySymbols(withSym).length, withSym[s1]);

// Proxy traps.
const log = [];
const p = new Proxy({ a: 1 }, {
  get(t, k, r) { if (typeof k === 'string') log.push('get:' + k); return Reflect.get(t, k, r); },
  has(t, k) { log.push('has:' + String(k)); return Reflect.has(t, k); },
  set(t, k, v) { log.push('set:' + String(k)); return Reflect.set(t, k, v); },
  deleteProperty(t, k) { log.push('del:' + String(k)); return Reflect.deleteProperty(t, k); },
  ownKeys(t) { log.push('ownKeys'); return Reflect.ownKeys(t); },
});
p.a; p.b = 2; 'a' in p; delete p.b; Object.keys(p);
console.log('proxy   ', log.join(' '));

// A proxy over a function is callable and constructible.
const fp = new Proxy(function (x) { return x * 2; }, {
  apply(t, thisArg, args) { return Reflect.apply(t, thisArg, args) + 1; },
});
console.log('applyTrp', fp(5));

// Reflect basics.
console.log('reflect ', Reflect.has({ x: 1 }, 'x'), Reflect.ownKeys({ a: 1, b: 2 }).join(','));
