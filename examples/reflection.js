// `Reflect`. Nothing in the corpus reached it, and four of its methods ignored
// the argument that distinguishes them from their `Object` counterparts.
const attempt = (f) => { try { return String(f()); } catch (e) { return e.constructor.name; } };

// `Reflect.set(target, key, value, receiver)` — the receiver is what a setter
// sees as `this`, and where a DATA property lands (28.1.13). It was ignored, so
// the setter ran against the target and the property was written there.
const dataTarget = { a: 1 };
const dataRecv = {};
Reflect.set(dataTarget, "a", 9, dataRecv);
console.log("set-data", dataTarget.a, dataRecv.a);
const setterTarget = { set v(x) { this.got = x; } };
const setterRecv = {};
Reflect.set(setterTarget, "v", 5, setterRecv);
console.log("set-accs", String(setterTarget.got), setterRecv.got);
// With no receiver the target is the receiver, and an inherited setter still
// runs against the object it was called on.
const plain = { set v(x) { this.got = x; } };
Reflect.set(plain, "v", 5);
const inherited = Object.create({ set s(x) { this.here = x; } });
Reflect.set(inherited, "s", 7);
console.log("set-dflt", plain.got, inherited.here);

// `Reflect.get(target, key, receiver)` — the receiver is what a getter sees.
console.log("get-recv", Reflect.get({ get v() { return this.tag; } }, "v", { tag: "R" }));

// `Reflect.construct(target, args, newTarget)` — the third argument decides
// which constructor's `prototype` the instance gets (28.1.2). It was ignored,
// so the result always inherited from the target.
class Made { constructor(x) { this.x = x; } }
class Brand {}
const crossed = Reflect.construct(Made, [1], Brand);
console.log("newTarget", crossed.x, crossed instanceof Brand, crossed instanceof Made);
const direct = Reflect.construct(Made, [3]);
console.log("nt-self ", direct.x, direct instanceof Made, Array.isArray(Reflect.construct(Array, [1, 2])));

// `Reflect.defineProperty` REPORTS success as a boolean where
// `Object.defineProperty` throws (28.1.3). It was propagating the throw.
const defTarget = {};
Object.defineProperty(defTarget, "locked", { value: 1, configurable: false });
console.log("define  ", Reflect.defineProperty({}, "x", { value: 1 }), Reflect.defineProperty(defTarget, "locked", { value: 2 }));
// A new property on a non-extensible object is refused the same way — and
// `Object.defineProperty` throws for it, which was not being checked at all.
console.log("extens  ", Reflect.defineProperty(Object.freeze({}), "z", { value: 1 }), attempt(() => Object.defineProperty(Object.freeze({}), "z", { value: 1 })));

// Every Reflect method requires an OBJECT target (28.1); a primitive was being
// accepted and silently producing nothing.
console.log("nonobj  ", ["get", "set", "has", "ownKeys", "getPrototypeOf", "defineProperty", "deleteProperty"]
  .map((m) => attempt(() => Reflect[m](1, "x", 2))).join(","));
// `Object.getPrototypeOf` coerces a primitive instead of throwing — the two
// must not be merged.
console.log("object  ", attempt(() => Object.getPrototypeOf(1) === Number.prototype));

// The rest of the surface, unchanged.
console.log("others  ", Reflect.has({ a: 1 }, "a"), Reflect.ownKeys([1, 2]).join(","), Reflect.apply(function (a) { return this.t + a; }, { t: "T" }, [1]));
console.log("protos  ", attempt(() => { const o = {}; Reflect.setPrototypeOf(o, null); return Reflect.getPrototypeOf(o); }), Reflect.isExtensible({}));

// Proxy TRAP INVARIANTS (10.5). A proxy may lie about most things, but not
// about a property the TARGET has pinned — that guarantee is what a membrane or
// a hardened-JS shim relies on to know a frozen thing stays frozen. None of the
// checks existed, so every one of these lies was accepted.
const pinned = () => {
  const o = {};
  Object.defineProperty(o, "nc", { value: 1, writable: false, configurable: false });
  return o;
};
const check = (f) => { try { f(); return "allowed"; } catch (e) { return e.constructor.name; } };
console.log("get     ", check(() => new Proxy(pinned(), { get: () => 2 }).nc), check(() => new Proxy(pinned(), { get: () => 1 }).nc));
console.log("set     ", check(() => { "use strict"; new Proxy(pinned(), { set: () => true }).nc = 2; }));
console.log("has     ", check(() => "nc" in new Proxy(pinned(), { has: () => false })));
console.log("delete  ", check(() => { "use strict"; delete new Proxy(pinned(), { deleteProperty: () => true }).nc; }));
console.log("gopd    ", check(() => Object.getOwnPropertyDescriptor(new Proxy(pinned(), { getOwnPropertyDescriptor: () => undefined }), "nc")));
console.log("ownKeys ", check(() => Object.keys(new Proxy(pinned(), { ownKeys: () => [] }))), check(() => Object.keys(new Proxy({}, { ownKeys: () => ["a", "a"] }))));
// A non-extensible target pins its key set, its prototype and its extensibility.
console.log("nonext  ", check(() => Object.keys(new Proxy(Object.preventExtensions({ a: 1 }), { ownKeys: () => ["a", "b"] }))),
  check(() => Reflect.isExtensible(new Proxy(Object.preventExtensions({}), { isExtensible: () => true }))));
console.log("proto   ", check(() => Object.getPrototypeOf(new Proxy(Object.preventExtensions(Object.create(null)), { getPrototypeOf: () => Array.prototype }))));
console.log("defineP ", check(() => { "use strict"; Object.defineProperty(new Proxy(Object.preventExtensions({}), { defineProperty: () => true }), "z", { value: 1 }); }));
// An honest proxy is untouched: the traps still receive their documented
// arguments and their results still stand.
const seen = [];
const target = { k: 1 };
const honest = new Proxy(target, {
  get(t, key, recv) { seen.push(["get", String(key), recv === honest].join(":")); return Reflect.get(t, key, recv); },
  set(t, key, value, recv) { seen.push(["set", String(key), value].join(":")); return Reflect.set(t, key, value, recv); },
  has(t, key) { seen.push("has:" + String(key)); return Reflect.has(t, key); },
  ownKeys(t) { seen.push("ownKeys"); return Reflect.ownKeys(t); },
});
honest.k; honest.j = 2; "k" in honest; Object.keys(honest);
console.log("honest  ", seen.join(","), target.j);
console.log("callable", new Proxy(function (a) { return a * 2; }, {})(21), new Proxy(class { constructor() { this.z = 1; } }, {}) && new (new Proxy(class { constructor() { this.z = 1; } }, {}))().z);

// OrdinarySetWithOwnDescriptor (10.1.9.2) when the RECEIVER is not the object
// the lookup started on. A data property is CREATED on the receiver via
// [[DefineOwnProperty]] — it is not assigned, and it does not land on the
// target. Routing it back through [[Set]] made a proxy receiver re-enter its own
// `set` trap forever, which broke the documented way to write a forwarding
// handler: `set(t, k, v, recv) { return Reflect.set(t, k, v, recv); }`.
const order = [];
const inner = {};
const fwd = new Proxy(inner, {
  set(t, k, v, recv) { order.push("set:" + k); return Reflect.set(t, k, v, recv); },
  defineProperty(t, k, d) { order.push("defineProperty:" + k); return Reflect.defineProperty(t, k, d); },
});
fwd.a = 1;
console.log("forward ", order.join(","), inner.a, fwd.a);
// The write lands on the receiver; the target keeps its own value.
const from = { x: 0 };
const onto = {};
console.log("receiver", Reflect.set(from, "x", 9, onto), from.x, onto.x, Object.hasOwn(onto, "x"));
// A setter on the target runs with `this` = receiver and defines nothing there.
const to2 = {};
Reflect.set({ set s(v) { this.got = v; } }, "s", 5, to2);
console.log("setter  ", to2.got, Object.hasOwn(to2, "s"));
// The receiver's own read-only slot, or its own accessor, refuses the write —
// reported as `false` rather than silently overwritten.
const ro = {};
Object.defineProperty(ro, "y", { value: 1, writable: false, configurable: true });
console.log("refused ", Reflect.set({ y: 0 }, "y", 7, ro), ro.y,
  Reflect.set({ z: 0 }, "z", 7, { get z() { return 3; } }));
// A getter-only property on the target's chain refuses too, and a primitive
// receiver can hold nothing.
console.log("no-setter", Reflect.set({ get g() { return 1; } }, "g", 2), Reflect.set({ q: 1 }, "q", 2, 5));
