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
