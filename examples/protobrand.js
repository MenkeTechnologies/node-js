// `Object.prototype.toString`'s brand (20.1.3.6 step 15) is a `Get` of
// `Symbol.toStringTag`, which WALKS the prototype chain. It was decided from
// the receiver's own kind instead, so an object inheriting from an intrinsic
// prototype reported `[object Object]` — and one inheriting from an ERROR
// prototype reported `[object Error]`, which is the opposite mistake: node
// brands an error by its `[[ErrorData]]` slot, not by what it inherits.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const brand = (o) => Object.prototype.toString.call(o);

// Borrowed from the chain, at any depth, and across the ES5 subclass pattern.
show("created      ", () => [brand(Object.create(Map.prototype)), brand(Object.create(Set.prototype)), brand(Object.create(Promise.prototype))]);
show("deep         ", () => brand(Object.create(Object.create(Set.prototype))));
show("es5-subclass ", () => { function F() {} F.prototype = Object.create(Map.prototype); return brand(new F()); });
show("class-chain  ", () => { class D extends Map {} return [brand(new D()), brand(Object.create(D.prototype))]; });
show("tag-read     ", () => [Object.create(Map.prototype)[Symbol.toStringTag], Object.create(Promise.prototype)[Symbol.toStringTag], Object.create(Error.prototype)[Symbol.toStringTag]]);
// Only the prototypes that REALLY carry the symbol. `Array`, `Date`, `RegExp`
// and the error hierarchy carry none, so inheriting from them borrows nothing.
show("no-tag       ", () => [brand(Object.create(Array.prototype)), brand(Object.create(Date.prototype)), brand(Object.create(RegExp.prototype))]);
show("error-chain  ", () => [brand(Object.create(Error.prototype)), brand(Object.create(TypeError.prototype)), brand(new Error("x"))]);
// …which is the same line `Error.isError` draws: the slot, not the chain.
show("is-error     ", () => [Error.isError(Object.create(Error.prototype)), Error.isError(new Error("x")), Error.isError(new TypeError("y"))]);
// A real exotic still brands by its own slot, and a typed array by its KIND —
// `%TypedArray%.prototype`'s tag is an accessor returning the specific one, so
// consulting the chain first would call every view `[object TypedArray]`.
show("exotics      ", () => [brand(new Map()), brand(new Set()), brand(Promise.resolve()), brand(new WeakMap())]);
show("views        ", () => [brand(new Uint8Array(1)), brand(new Float64Array(1)), brand(new DataView(new ArrayBuffer(4))), brand(Buffer.from([1]))]);
show("null-proto   ", () => brand(Object.create(null)));
// An OWN tag wins over the inherited one — when the assignment is allowed.
show("own-tag      ", () => { const o = { [Symbol.toStringTag]: "X" }; return [brand(o), o[Symbol.toStringTag]]; });
show("own-getter   ", () => { const o = Object.create(Map.prototype); Object.defineProperty(o, Symbol.toStringTag, { get() { return "G"; } }); return brand(o); });
// `Map.prototype[Symbol.toStringTag]` is NON-WRITABLE, and an inherited
// non-writable data property refuses an assignment on every object below it
// (10.1.9.2) — so the write is dropped and the brand does not change.
show("write-refused", () => { const o = Object.create(Map.prototype); o[Symbol.toStringTag] = "X"; return [o[Symbol.toStringTag], o.hasOwnProperty(Symbol.toStringTag), brand(o)]; });
show("write-strict ", () => { "use strict"; const o = Object.create(Map.prototype); try { o[Symbol.toStringTag] = "X"; return "no throw"; } catch (e) { return e.constructor.name; } });
show("real-receiver", () => { const m = new Map(); m[Symbol.toStringTag] = "X"; return [m[Symbol.toStringTag], brand(m)]; });
// `defineProperty` is not an assignment and is not refused.
show("define-ok    ", () => { const o = Object.create(Map.prototype); Object.defineProperty(o, Symbol.toStringTag, { value: "D", configurable: true }); return [o[Symbol.toStringTag], brand(o)]; });
// The other non-writable inherited members behave the same way, and a WRITABLE
// inherited member still accepts the write.
show("unscopables  ", () => { const a = []; a[Symbol.unscopables] = "X"; return [typeof a[Symbol.unscopables], a.hasOwnProperty(Symbol.unscopables)]; });
show("date-toprim  ", () => { const d = new Date(0); d[Symbol.toPrimitive] = "X"; return typeof d[Symbol.toPrimitive]; });
show("writable-ok  ", () => { const o = Object.create(Map.prototype); o.get = () => 1; return [o.get(), o.hasOwnProperty("get")]; });
show("plain-object ", () => { const o = {}; o[Symbol.toStringTag] = "X"; return [o[Symbol.toStringTag], brand(o)]; });
