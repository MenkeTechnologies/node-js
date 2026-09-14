// The mirror of `protodetach.js`: an ordinary object whose CHAIN reaches an
// intrinsic prototype resolves that prototype's members through it. This is the
// ES5 subclassing pattern — `F.prototype = Object.create(Array.prototype)` —
// and none of it worked, because the owner was decided from the receiver's KIND
// and an ordinary object's kind is `Object`.
//
// The methods themselves were already generic over their receiver: calling one
// explicitly (`Array.prototype.push.call({length: 0}, 1)`) worked the whole
// time. Only the lookup was missing.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// The explicit form, which is what the resolved lookup has to agree with.
show("explicit-call", () => { const o = { length: 0 }; Array.prototype.push.call(o, 1, 2); return [o.length, o[0], o[1]]; });
show("explicit-join", () => Array.prototype.join.call({ length: 2, 0: "a", 1: "b" }, "-"));
// Read, `in` and CALL all resolve through the chain, and agree.
show("read         ", () => { const o = Object.create(Array.prototype); return [typeof o.push, typeof o.join, typeof o.map, typeof o.values]; });
show("in           ", () => { const o = Object.create(Array.prototype); return ["push" in o, "join" in o, "hasOwnProperty" in o]; });
show("call         ", () => { const o = Object.create(Array.prototype); o.length = 0; o.push(1, 2); return [o.length, o[0], o[1], o.join("-")]; });
// It is still NOT an array: no exotic storage, no `length` bookkeeping of its
// own, and `Object.prototype.toString` reports a plain object.
show("not-an-array ", () => { const o = Object.create(Array.prototype); o.length = 0; o.push(1); return [Array.isArray(o), Object.prototype.toString.call(o), Object.keys(o)]; });
// …but `instanceof` follows the chain, which is the point of the pattern.
show("instanceof   ", () => { const o = Object.create(Array.prototype); return [o instanceof Array, o instanceof Object]; });
show("es5-subclass ", () => { function F() { this.length = 0; } F.prototype = Object.create(Array.prototype); F.prototype.constructor = F; const f = new F(); f.push(1, 2); return [f.length, f[0], f.join("-"), f instanceof F, f instanceof Array, Array.isArray(f)]; });
// `Symbol.iterator` comes through too, so the object is iterable by length.
show("iterate      ", () => { const o = Object.create(Array.prototype); o.length = 2; o[0] = "a"; o[1] = "b"; return [...o]; });
// Other intrinsics resolve the same way.
show("map-proto    ", () => { const o = Object.create(Map.prototype); return [typeof o.get, typeof o.has, typeof o.set]; });
show("regexp-proto ", () => { const o = Object.create(RegExp.prototype); return [typeof o.test, typeof o.exec]; });
show("promise-proto", () => { const o = Object.create(Promise.prototype); return typeof o.then; });
show("error-proto  ", () => { const o = Object.create(Error.prototype); o.name = "E"; o.message = "m"; return [typeof o.toString, o.toString()]; });
// An own property still shadows, and a deeper chain still reaches.
show("own-shadows  ", () => { const o = Object.create(Array.prototype); o.join = () => "own"; return o.join(); });
show("mid-chain    ", () => { const mid = Object.create(Array.prototype); mid.extra = 1; const o = Object.create(mid); o.length = 1; o[0] = "z"; return [o.join(), o.extra, o instanceof Array]; });
show("deep-create  ", () => typeof Object.create(Object.create(Array.prototype)).join);
