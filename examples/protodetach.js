// A builtin's methods are synthesized from the receiver's KIND rather than
// found on its chain, so replacing the prototype could not take them away:
// `Object.setPrototypeOf(a, {})` left `a.join` a function and `a.join()`
// returning "1,2" where node reports `undefined` and a TypeError. The exotic
// STORAGE is unaffected either way, which is the part that stays true.
//
// The not-iterable message is not pinned here: node names the source
// EXPRESSION (`a is not iterable`) where this frontend renders the value, which
// is a separate divergence recorded in BUGS.md.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name); }
};
const detach = (v) => { Object.setPrototypeOf(v, {}); return v; };

// The kind's own methods go; `Object.prototype`'s stay, because the plain
// object put in their place inherits from it.
show("array-read   ", () => { const a = detach([1, 2]); return [typeof a.join, typeof a.map, typeof a.toString, typeof a.hasOwnProperty]; });
show("array-in     ", () => { const a = detach([1, 2]); return ["join" in a, "toString" in a, "hasOwnProperty" in a]; });
show("array-call   ", () => { const a = detach([1, 2]); try { return a.join(); } catch (e) { return e.constructor.name; } });
// `Array.prototype.toString` is gone, so `a + ''` reaches the Object one.
show("array-string ", () => { const a = detach([1, 2]); return [a + "", String(a)]; });
// Storage is untouched: still an array, still indexable, still JSON-able.
show("still-array  ", () => { const a = detach([1, 2]); return [Array.isArray(a), a.length, a[0], JSON.stringify(a)]; });
// Every other exotic behaves the same way.
show("map          ", () => { const m = detach(new Map([[1, 2]])); return [typeof m.get, typeof m.has, typeof m.hasOwnProperty]; });
show("regexp       ", () => { const r = detach(/a/); return [typeof r.test, typeof r.exec]; });
show("promise      ", () => { const p = detach(Promise.resolve(1)); return typeof p.then; });
show("function     ", () => { const f = detach(() => 1); return [typeof f.call, typeof f.bind, f()]; });
show("boxed-string ", () => { const s = detach(Object(" x ")); return typeof s.trim; });
// A null prototype takes `Object.prototype`'s too, and so does a replacement
// that is itself null-prototyped — there the chain reaches NEITHER intrinsic,
// which is the case that tells the two halves of the rule apart.
show("null-proto   ", () => { const a = [1, 2]; Object.setPrototypeOf(a, null); return [typeof a.join, typeof a.hasOwnProperty]; });
show("bare-proto   ", () => { const a = [1, 2]; Object.setPrototypeOf(a, Object.create(null)); return [typeof a.join, typeof a.hasOwnProperty, typeof a.toString, "hasOwnProperty" in a]; });
show("bare-plain   ", () => { const o = {}; Object.setPrototypeOf(o, Object.create(null)); return [typeof o.hasOwnProperty, typeof o.toString]; });
// `Symbol.iterator` comes from the intrinsic prototype, so iterating stops.
show("spread       ", () => { const a = detach([1, 2]); try { return [...a]; } catch (e) { return e.constructor.name; } });
show("for-of       ", () => { const a = detach([1, 2]); try { for (const x of a); return "ok"; } catch (e) { return e.constructor.name; } });
show("destructure  ", () => { const a = detach([1, 2]); try { const [x] = a; return x; } catch (e) { return e.constructor.name; } });
show("param-pattern", () => { const a = detach([1, 2]); const f = ([x]) => x; try { return f(a); } catch (e) { return e.constructor.name; } });
show("map-iterate  ", () => { const m = detach(new Map([[1, 2]])); try { return [...m]; } catch (e) { return e.constructor.name; } });
// `Array.from` reads it as an ARRAY-LIKE instead, which still works.
show("array-from   ", () => Array.from(detach([1, 2])));
// A replacement that DEFINES the method supplies it, and restoring the link
// brings the intrinsics back.
show("replacement  ", () => { const a = [1, 2]; Object.setPrototypeOf(a, { join: () => "C" }); return a.join(); });
show("restore      ", () => { const a = detach([1, 2]); Object.setPrototypeOf(a, Array.prototype); return [a.join("-"), [...a]]; });
// Subclassing must NOT be affected: the chain still reaches the intrinsic.
show("subclass     ", () => { class D extends Array {} const d = new D(); d.push(1, 2); return [typeof d.join, d.join("-"), Array.isArray(d), [...d]]; });
show("subclass-map ", () => { class M extends Map {} const m = new M(); m.set(1, 2); return [typeof m.get, m.get(1)]; });
show("deep-subclass", () => { class A extends Array {} class B extends A {} const b = new B(); b.push(3); return [typeof b.join, b.join()]; });
