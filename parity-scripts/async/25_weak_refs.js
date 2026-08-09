// WeakMap / WeakSet / WeakRef semantics that do not depend on GC timing.
const k1 = {};
const k2 = { tag: 2 };
const wm = new WeakMap([[k1, "one"]]);
wm.set(k2, "two");
console.log(wm.get(k1), wm.get(k2), wm.get({}));
console.log(wm.has(k1), wm.has({}), wm.delete(k1), wm.has(k1), wm.delete(k1));
console.log(wm instanceof WeakMap, Object.prototype.toString.call(wm));
console.log(JSON.stringify(Object.keys(wm)), JSON.stringify(wm));

const ws = new WeakSet([k1]);
ws.add(k2);
console.log(ws.has(k1), ws.has(k2), ws.has({}), ws.delete(k2), ws.has(k2));

// Non-object keys are rejected.
try { new WeakMap().set(1, "x"); } catch (e) { console.log("wm-key", e.constructor.name); }
try { new WeakSet().add("s"); } catch (e) { console.log("ws-key", e.constructor.name); }

// A live referent is always dereferenceable.
const target = { alive: true };
const ref = new WeakRef(target);
console.log(ref.deref() === target, ref.deref().alive);
console.log(typeof WeakRef, typeof FinalizationRegistry);
const reg = new FinalizationRegistry(() => {});
console.log(typeof reg.register, typeof reg.unregister);

// Weak collections hold no enumerable state.
console.log(wm.size, ws.size, JSON.stringify(Object.getOwnPropertyNames(wm)));
