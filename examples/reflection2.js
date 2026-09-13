// `Reflect`'s argument handling and `[[SetPrototypeOf]]`'s extensibility check.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// `Reflect.apply`/`construct` take an ARRAY-LIKE, not an iterable
// (CreateListFromArrayLike, 7.3.18). The iterator was used instead, so an
// array-like produced an empty list and a primitive produced one too, rather
// than the TypeError node raises.
show("array-like  ", () => Reflect.apply(Math.max, null, { length: 2, 0: 1, 1: 5 }));
show("arguments   ", () => { function f() { return Reflect.apply(Math.max, null, arguments); } return f(3, 9, 2); });
show("array       ", () => Reflect.apply(Math.max, null, [1, 3, 2]));
show("construct   ", () => { class C { constructor(a, b) { this.s = a + b; } } return Reflect.construct(C, { length: 2, 0: 1, 1: 2 }).s; });
show("holes       ", () => Reflect.apply(Array.prototype.join, [1, 2], []));
for (const bad of [1, "s", null, undefined, true]) {
  show(("non-object " + JSON.stringify(bad)).padEnd(24), () => Reflect.apply(Math.max, null, bad));
}
// A `this` of null/undefined is dropped, not passed through.
show("this        ", () => [Reflect.apply(function () { return this; }, { v: 7 }, []).v,
  Reflect.apply(function () { return this; }, null, []) === undefined]);

// 10.1.2.1: a NON-EXTENSIBLE object refuses a prototype change — unless the new
// prototype is what it already has, which changes nothing. Both forms reported
// success and rewrote the link.
show("reflect-diff", () => Reflect.setPrototypeOf(Object.freeze({}), { a: 1 }));
show("reflect-same", () => Reflect.setPrototypeOf(Object.freeze({}), Object.prototype));
show("reflect-arr ", () => Reflect.setPrototypeOf(Object.freeze([]), Array.prototype));
show("reflect-prev", () => Reflect.setPrototypeOf(Object.preventExtensions({}), { a: 1 }));
// `Object.setPrototypeOf` throws where `Reflect` reports false, and names the
// receiver by its brand — a null-prototype object has no constructor to name.
show("object-diff ", () => Object.setPrototypeOf(Object.freeze({}), { a: 1 }));
show("object-same ", () => { const o = Object.freeze({}); return Object.setPrototypeOf(o, Object.prototype) === o; });
show("null-same   ", () => { const o = Object.preventExtensions(Object.create(null)); return Object.setPrototypeOf(o, null) === o; });
show("null-diff   ", () => Object.setPrototypeOf(Object.preventExtensions(Object.create(null)), {}));
// The `__proto__` setter runs the same operation, and throws in SLOPPY code too.
show("setter-frozen", () => { const o = Object.freeze({}); o.__proto__ = { a: 1 }; return "no throw"; });
show("setter-same  ", () => { const o = Object.freeze({}); o.__proto__ = Object.prototype; return "no throw"; });
show("setter-ok    ", () => { const o = {}; o.__proto__ = { a: 2 }; return o.a; });
show("extensible   ", () => { const o = {}; Object.setPrototypeOf(o, { a: 1 }); return o.a; });

// The rest of the surface, unchanged and pinned so it stays that way.
show("members     ", () => Object.getOwnPropertyNames(Reflect).sort());
show("get-receiver", () => { const o = { get a() { return this.v; }, v: 1 }; return Reflect.get(o, "a", { v: 9 }); });
show("set-receiver", () => { const o = { set a(v) { this.got = v; } }; const r = {}; Reflect.set(o, "a", 5, r); return [r.got, o.got]; });
show("ownKeys     ", () => { const s = Symbol("s"); return Reflect.ownKeys({ b: 1, [s]: 2, 1: 3 }).map(String); });
show("newTarget   ", () => { class A {} class B {} const i = Reflect.construct(A, [], B); return [Object.getPrototypeOf(i) === B.prototype, i instanceof B]; });
show("refusals    ", () => [Reflect.defineProperty(Object.freeze({}), "x", { value: 1 }),
  Reflect.deleteProperty(Object.freeze({ c: 1 }), "c"), Reflect.isExtensible(Object.freeze({}))]);
show("non-object  ", () => Reflect.get(1, "x"));
