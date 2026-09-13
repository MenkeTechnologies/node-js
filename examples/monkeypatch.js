// Patching an intrinsic prototype — how every polyfill installs itself. None of
// it worked: the assignment threw nothing, `Object.isExtensible` said true, and
// the value read back `undefined`. The prototypes are namespace handles rather
// than objects on the chain, so the write landed in a side table no read
// consulted.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// The prototype itself reads back the assignment, and so does an instance.
show("read-proto   ", () => { Array.prototype.zq = "v"; const r = [Array.prototype.zq, [].zq, [1, 2].zq]; delete Array.prototype.zq; return r; });
// In STRICT code a refused write throws, so the silent drop was observable as
// the absence of a TypeError as well as the wrong read.
show("strict-write ", () => { "use strict"; Array.prototype.zq = "v"; const r = [].zq; delete Array.prototype.zq; return r; });
// A plain assignment creates a writable/enumerable/configurable data property.
show("descriptor   ", () => { Array.prototype.zq = "v"; const d = Object.getOwnPropertyDescriptor(Array.prototype, "zq"); delete Array.prototype.zq; return d; });
// …so it enumerates, where the intrinsic members (non-enumerable) do not.
show("enumerates   ", () => { Array.prototype.zq = "v"; const k = Object.keys(Array.prototype); const own = Object.getOwnPropertyNames(Array.prototype).includes("zq"); delete Array.prototype.zq; return [k, own]; });
// `defineProperty` is the form a careful polyfill uses, precisely to AVOID the
// enumerable property a bare assignment makes.
show("defineProp   ", () => { Object.defineProperty(Array.prototype, "zd", { value: "d", configurable: true }); const r = [1][8 - 8 + 0] === 1 ? [1].zd : null; delete Array.prototype.zd; return r; });
// A patched METHOD is callable, with the receiver as `this`. The read resolved
// it and the call reported "is not a function" — the two paths disagreeing.
show("call-array   ", () => { Array.prototype.zm = function () { return this.length * 2; }; const r = [1, 2, 3].zm(); delete Array.prototype.zm; return r; });
show("call-string  ", () => { String.prototype.zm = function () { return "Z" + this; }; const r = "a".zm(); delete String.prototype.zm; return r; });
show("call-object  ", () => { Object.prototype.zm = function () { return "O"; }; const r = [({}).zm(), (function () {}).zm()]; delete Object.prototype.zm; return r; });
show("call-map     ", () => { Map.prototype.zm = function () { return this.size; }; const r = new Map([[1, 1]]).zm(); delete Map.prototype.zm; return r; });
show("call-promise ", () => { Promise.prototype.zm = function () { return "P"; }; const r = Promise.resolve().zm(); delete Promise.prototype.zm; return r; });
// Overriding an EXISTING intrinsic method — the patch has to win over the
// builtin, which is what makes a shim replaceable.
show("override     ", () => { const orig = Array.prototype.join; Array.prototype.join = () => "X"; const r = [1, 2].join(); Array.prototype.join = orig; return [r, [1, 2].join()]; });
// An OWN property on the receiver still shadows the patched prototype.
show("own-shadows  ", () => { Object.prototype.zq = "proto"; const o = { zq: "own" }; const r = [o.zq, ({}).zq]; delete Object.prototype.zq; return r; });
// `delete` really removes it — it answered true and left the patch in place,
// so a shim could not uninstall itself.
show("delete       ", () => { Array.prototype.zq = "v"; const before = [].zq; const d = delete Array.prototype.zq; return [before, d, [].zq === undefined, Object.keys(Array.prototype)]; });
// A null-prototype object inherits none of it.
show("null-proto   ", () => { Object.prototype.zq = "v"; const r = [Object.create(null).zq === undefined, ({}).zq]; delete Object.prototype.zq; return r; });
