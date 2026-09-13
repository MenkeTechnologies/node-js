// A Proxy's refusing traps, and the integrity operations over one. Each of
// these was a place the proxy OBJECT was inspected or mutated directly, so the
// handler never saw the operation at all.
"use strict";
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
// A handler that records every trap it is asked for, in order.
const recording = (target, seen) => new Proxy(target, new Proxy({}, {
  get: (_, trap) => (...a) => { seen.push(trap); return Reflect[trap](...a); },
}));

// `Object.freeze`/`seal` over a proxy is SetIntegrityLevel (7.3.15) — a
// sequence of traps, not a flag on the proxy. It ran none of them, so the
// target was left untouched.
show("freeze-traps ", () => { const seen = []; Object.freeze(recording({ a: 1 }, seen)); return seen; });
show("seal-traps   ", () => { const seen = []; Object.seal(recording({ a: 1 }, seen)); return seen; });
// …and `isFrozen`/`isSealed` is TestIntegrityLevel (7.3.16), likewise.
show("isFrozen     ", () => { const seen = []; const p = recording({ a: 1 }, seen); Object.freeze(p); seen.length = 0; return [Object.isFrozen(p), seen]; });
show("levels       ", () => { const p = new Proxy({ a: 1 }, {}); Object.freeze(p); return [Object.isFrozen(p), Object.isSealed(p), Object.isExtensible(p)]; });
show("sealed-only  ", () => { const p = new Proxy({ a: 1 }, {}); Object.seal(p); return [Object.isFrozen(p), Object.isSealed(p)]; });
// The target really is frozen afterwards, which is the point.
show("target       ", () => { const t = { a: 1 }; Object.freeze(new Proxy(t, {})); return [Object.isFrozen(t), Object.getOwnPropertyDescriptor(t, "a").writable]; });
// An ACCESSOR loses `configurable` but has no `writable` to lose.
show("accessor     ", () => { const t = { get g() { return 1; } }; Object.freeze(new Proxy(t, {})); const d = Object.getOwnPropertyDescriptor(t, "g"); return [d.configurable, "writable" in d]; });
// An extensible proxy is neither, whatever its keys report.
show("extensible   ", () => { const p = new Proxy({}, {}); return [Object.isFrozen(p), Object.isSealed(p)]; });

// A trap that returns FALSISH refused the operation. The return value was
// discarded, so every one of these looked like a success.
show("set          ", () => { const p = new Proxy({}, { set: () => false }); p.x = 1; return "no throw"; });
show("delete       ", () => { const p = new Proxy({ a: 1 }, { deleteProperty: () => false }); delete p.a; return "no throw"; });
show("define       ", () => { const p = new Proxy({}, { defineProperty: () => false }); Object.defineProperty(p, "x", { value: 1 }); return "no throw"; });
// `Reflect` reports the same refusals as `false` rather than throwing.
show("reflect      ", () => [
  Reflect.set(new Proxy({}, { set: () => false }), "x", 1),
  Reflect.deleteProperty(new Proxy({ a: 1 }, { deleteProperty: () => false }), "a"),
  Reflect.defineProperty(new Proxy({}, { defineProperty: () => false }), "x", { value: 1 }),
]);
// A truthy trap still succeeds, and the target sees it.
show("accepted     ", () => { const t = {}; const p = new Proxy(t, { set: (o, k, v) => { o[k] = v; return true; } }); p.x = 5; return [t.x, delete new Proxy({ a: 1 }, {}).a]; });

// The `in`, descriptor and method reads this session changed all still route
// through the handler on a proxy.
show("in-traps     ", () => { const seen = []; const p = recording({ a: 1 }, seen); const r = ["a" in p, "toString" in p, "nope" in p]; return [r, seen]; });
show("fn-proxy     ", () => { const p = new Proxy(function f(a) {}, {}); return [typeof p.hasOwnProperty, typeof p.call, p.name, Object.getOwnPropertyNames(p).sort()]; });
show("revoked      ", () => { const { proxy, revoke } = Proxy.revocable({ a: 1 }, {}); revoke(); return "a" in proxy; });
