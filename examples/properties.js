// Property ordering, integrity levels and the array exotic object. The
// enumeration half of this is where `Object.keys` was found running getters, so
// the order and the flags are pinned here rather than assumed.
const o = { b: 1, 2: 2, a: 3, 1: 4, [Symbol('s')]: 5, '01': 6 };
console.log('order   ', Object.keys(o).join(','));
console.log('reflect ', Reflect.ownKeys(o).length);

const frozen = Object.freeze({ x: 1, nested: { y: 2 } });
frozen.x = 99; frozen.nested.y = 99; delete frozen.x;
console.log('freeze  ', frozen.x, frozen.nested.y, Object.isFrozen(frozen), Object.isFrozen(frozen.nested));

const sealed = Object.seal({ a: 1 });
sealed.a = 2; sealed.b = 3; delete sealed.a;
console.log('seal    ', sealed.a, sealed.b, Object.isSealed(sealed), Object.isExtensible(sealed));

// Array exotics: length is writable and truncates.
const arr = [1, 2, 3, 4];
arr.length = 2;
console.log('length  ', arr.join(','), arr.length);
arr[5] = 'x';
console.log('sparse  ', arr.length, JSON.stringify(arr), 3 in arr);

// defineProperty on an array index goes through the exotic [[DefineOwnProperty]].
const a2 = [1, 2, 3];
Object.defineProperty(a2, 1, { value: 'two', enumerable: true, writable: true, configurable: true });
console.log('defArr  ', a2.join(','), a2.length);

// Non-writable then assignment is a silent no-op in sloppy mode.
const nw = {};
Object.defineProperty(nw, 'k', { value: 1, writable: false, configurable: false });
nw.k = 2;
console.log('nonwrit ', nw.k);

// preventExtensions
const pe = Object.preventExtensions({ p: 1 });
pe.q = 2;
console.log('prevent ', pe.q, Object.isExtensible(pe), Object.isSealed(pe));

// getOwnPropertyDescriptors round-trips through create.
const src = { get g() { return 'G'; }, d: 1 };
const clone = Object.create(Object.getPrototypeOf(src), Object.getOwnPropertyDescriptors(src));
console.log('descrs  ', clone.g, clone.d, typeof Object.getOwnPropertyDescriptor(clone, 'g').get);

// Enumeration must not RUN a getter. 20.1.2.17 -> 7.3.23 reads
// `[[GetOwnProperty]]` for the enumerable flag and never `[[Get]]` when only
// keys are wanted, so a getter with side effects has to stay untouched here —
// `Object.keys` used to invoke it. `entries` and spread do read values.
let reads = 0;
const watched = { get g() { reads++; return 1; }, plain: 2 };
Object.keys(watched);
Object.getOwnPropertyNames(watched);
for (const _k in watched) { /* for-in reads names only */ }
console.log('nogetter', reads);
Object.entries(watched);
console.log('getter  ', reads);

// The same rule through a Proxy: `Object.keys` runs `ownKeys` and
// `getOwnPropertyDescriptor`, never `get`.
const traps = [];
const px = new Proxy({ a: 1, b: 2 }, {
  get(t, k, r) { traps.push('get'); return Reflect.get(t, k, r); },
  ownKeys(t) { traps.push('ownKeys'); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) { traps.push('gopd'); return Reflect.getOwnPropertyDescriptor(t, k); },
});
Object.keys(px);
console.log('proxykeys', traps.join(','));

// ValidateAndApplyPropertyDescriptor (10.1.6.3). None of this validation
// existed — every defineProperty was applied unconditionally.
const nc = {};
Object.defineProperty(nc, 'k', { value: 1 });
const attempt = (f) => { try { f(); return 'NO THROW'; } catch (e) { return e.constructor.name; } };
console.log('redefine', attempt(() => Object.defineProperty(nc, 'k', { value: 2 })));
console.log('toacc   ', attempt(() => Object.defineProperty(nc, 'k', { get() { return 3; } })));
// Redefining with the SAME value is allowed even when non-configurable.
console.log('samevalue', attempt(() => Object.defineProperty(nc, 'k', { value: 1 })));
// Non-configurable but writable may still change value, and may be made
// non-writable — the one-way door.
const nw2 = {};
Object.defineProperty(nw2, 'w', { value: 1, writable: true });
Object.defineProperty(nw2, 'w', { value: 2 });
console.log('ncwrit  ', nw2.w, attempt(() => Object.defineProperty(nw2, 'w', { writable: false })));

// An OMITTED field leaves the existing attribute alone; it does not reset it to
// false. This is the everyday pattern that made the bug matter: marking a
// property non-enumerable also stripped writable and configurable from it.
const keep = { hidden: 1, shown: 2 };
Object.defineProperty(keep, 'hidden', { enumerable: false });
keep.hidden = 42;
console.log('omitted ', Object.keys(keep).join(','), keep.hidden);
console.log('desc    ', JSON.stringify(Object.getOwnPropertyDescriptor(keep, 'hidden')));
// A brand-new property still defaults every absent field to false.
const fresh = {};
Object.defineProperty(fresh, 'n', { value: 3 });
console.log('fresh   ', JSON.stringify(Object.getOwnPropertyDescriptor(fresh, 'n')));

// An array's `length` is the exotic own property whose write resizes it; going
// through the ordinary path stored a shadowing key and left the elements alone.
const trunc = [1, 2, 3];
Object.defineProperty(trunc, 'length', { value: 1 });
console.log('length  ', trunc.join(','), trunc.length);

// Stringifying a proxy used to overflow the stack and abort the process.
// `Get(proxy, 'toString')` yields a thunk already bound to the TARGET, and the
// call path was invoking it with the PROXY as `this` — which the bound-method
// arm prefers over the thunk's own receiver, so it called straight back in.
// These three generics resolve by the target's kind, so the thunk is invoked
// with no receiver override.
console.log('px-str  ', String(new Proxy({}, {})), String(new Proxy([1, 2], {})));
console.log('px-concat', new Proxy({}, {}) + '', new Proxy([1, 2], {}) + '');
console.log('px-nested', String(new Proxy(new Proxy({}, {}), {})));
// The target's own valueOf/toString still drive the conversion.
console.log('px-valueOf', new Proxy({ valueOf() { return 5; } }, {}) + 1);
console.log('px-toString', new Proxy({ toString() { return 'TS'; } }, {}) + '');
// A `get` trap still intercepts the lookup ahead of all of that.
console.log('px-trap ', String(new Proxy({}, { get: (t, k) => (k === 'toString' ? () => 'TRAPPED' : t[k]) })));
// The other proxy paths are unaffected.
console.log('px-other', new Proxy([1, 2], {}).join('-'), new Proxy({ a: 1 }, {}).hasOwnProperty('a'), new Proxy(function (a) { return a * 2; }, {})(21));
