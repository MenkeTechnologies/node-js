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

// A write the object REFUSES. With no own property the INHERITED one decides:
// a non-writable data property up the chain blocks the write rather than being
// shadowed, including one on a frozen prototype. Only own attributes were
// consulted, so an own property got created that node refuses to create.
const lockedBase = {};
Object.defineProperty(lockedBase, "ro", { value: "base", writable: false, configurable: true });
const heir = Object.create(lockedBase);
heir.ro = "child";
console.log("inh-ro  ", heir.ro, Object.prototype.hasOwnProperty.call(heir, "ro"));
const frozenBase = Object.freeze({ f: 1 });
const heir2 = Object.create(frozenBase);
heir2.f = 2;
console.log("inh-froz", heir2.f, Object.prototype.hasOwnProperty.call(heir2, "f"));
// A WRITABLE inherited property is shadowed as usual, and an inherited SETTER
// still runs — neither blocks.
const openBase = { rw: "base" };
const heir3 = Object.create(openBase);
heir3.rw = "child";
const withSetter = Object.create({ set s(v) { this.got = v; } });
withSetter.s = "v";
console.log("inh-rw  ", heir3.rw, Object.prototype.hasOwnProperty.call(heir3, "rw"), withSetter.got);

// Refusing is SILENT in sloppy mode and a TypeError in strict code — the
// ASSIGNMENT SITE decides, not the object. Every refusal used to be silent, so
// `'use strict'` did not catch a write to a frozen object, which is most of the
// reason to freeze one.
const iced = Object.freeze({ a: 1 });
const nonext = Object.preventExtensions({ c: 1 });
const getterOnly = {};
Object.defineProperty(getterOnly, "g", { get: () => 1 });
const tryWrite = (f) => { try { f(); return "silent"; } catch (e) { return e.constructor.name; } };
console.log("sloppy  ", tryWrite(() => { iced.a = 2; }), tryWrite(() => { iced.z = 2; }), tryWrite(() => { nonext.z = 2; }), tryWrite(() => { getterOnly.g = 2; }));
(function () {
  "use strict";
  console.log("strict  ", tryWrite(() => { iced.a = 2; }), tryWrite(() => { iced.z = 2; }), tryWrite(() => { nonext.z = 2; }), tryWrite(() => { getterOnly.g = 2; }));
  console.log("strict2 ", tryWrite(() => { heir.ro = "x"; }), tryWrite(() => { heir2.f = 3; }));
  // A sealed property that is still WRITABLE assigns normally in either mode.
  const sealed = Object.seal({ b: 1 });
  console.log("sealed  ", tryWrite(() => { sealed.b = 2; }), sealed.b, tryWrite(() => { sealed.z = 1; }));
  // And a setter that EXISTS runs rather than being refused — including a
  // private one, where the refusal branch must not swallow a successful write.
  const sink = {};
  Object.defineProperty(sink, "v", { set(x) { this.got = x; }, get() { return "G"; } });
  sink.v = "written";
  class Priv { get #p() { return 3; } set #p(x) { this.seen = x; } write(x) { this.#p = x; return this.seen; } }
  console.log("setters ", sink.got, sink.v, new Priv().write(4));
})();

{
// A getter that THROWS has to propagate. Five paths read properties through one
// infallible helper, and every one of them swallowed the exception and reported
// the property as absent — `JSON.stringify` produced `{}` for an object whose
// only getter threw, which silently loses data rather than failing.
const boom = () => ({ get a() { throw new Error("boom"); } });
const caught = (f) => { try { f(); return "no-throw"; } catch (e) { return e.message; } };
console.log("entries ", caught(() => Object.entries(boom())), caught(() => Object.values(boom())));
console.log("copy    ", caught(() => Object.assign({}, boom())), caught(() => ({ ...boom() })));
console.log("json    ", caught(() => JSON.stringify(boom())));
// `Object.keys` does NOT read values, so it still succeeds.
console.log("keys    ", Object.keys(boom()).join(","));
// A getter that returns normally is still run exactly once, in declaration order.
const order = [];
const probed = { get a() { order.push("a"); return 1; }, get b() { order.push("b"); return 2; } };
console.log("runs    ", JSON.stringify(probed), order.join(","));

// 10.4.2.1: defining an ACCESSOR at an index past the end extends the array,
// exactly as defining a data property there does. Only the data path grew it,
// so the accessor sat in the side table with `length` unchanged — and being out
// of range, `Object.keys` and `JSON.stringify` never saw the index.
const grown = [1];
Object.defineProperty(grown, "1", { get() { return 9; }, enumerable: true, configurable: true });
console.log("grow    ", grown.length, grown[1], JSON.stringify(grown), Object.keys(grown).join(","));
const sparse = [1];
Object.defineProperty(sparse, "4", { get() { return 9; }, enumerable: true, configurable: true });
console.log("sparse  ", sparse.length, JSON.stringify(sparse));
// An in-range index is replaced rather than appended.
const replaced = [1, 2];
Object.defineProperty(replaced, "1", { get() { return 9; }, enumerable: true, configurable: true });
console.log("replace ", replaced.length, replaced[1], JSON.stringify(replaced));

// 23.1.3.23: `push` defines each element through `CreateDataPropertyOrThrow` and
// then SETS `length`, so a non-extensible array refuses it and so does one whose
// `length` is non-writable. Both appended to the backing vector regardless, so
// sealing an array did not seal it.
const sealed = [1];
Object.seal(sealed);
console.log("sealed  ", caught(() => sealed.push(2)) !== "no-throw", sealed.length,
  Object.isSealed(sealed), Object.isExtensible(sealed));
const prevented = [1];
Object.preventExtensions(prevented);
console.log("prevent ", caught(() => prevented.push(2)) !== "no-throw", prevented.length);
const pinned = [1, 2];
Object.defineProperty(pinned, "length", { writable: false });
console.log("pinned  ", caught(() => pinned.push(3)) !== "no-throw", pinned.length);
// Writing an EXISTING index of a sealed array still works — sealing forbids
// adding and removing, not assigning.
sealed[0] = 9;
console.log("write   ", sealed[0], [1, 2].push(3));
}
