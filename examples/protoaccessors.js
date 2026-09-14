// An intrinsic prototype's ACCESSOR members. `PROTO_MEMBERS` knew their names —
// `'size' in new Map()` was already true — but nothing recorded that they are
// accessors, so `Object.getOwnPropertyDescriptor(Map.prototype, 'size')` was
// `undefined` and reading one OFF THE PROTOTYPE answered `undefined` where node
// runs the getter, fails its brand check and throws.
//
// A function RECEIVER is avoided below: its rendering quotes source text this
// frontend does not retain (see BUGS.md), which would pin a known divergence.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const getter = (C, k) => Object.getOwnPropertyDescriptor(C.prototype, k).get;

// The descriptor is an ACCESSOR descriptor, with no setter and no `value`.
show("descriptor   ", () => { const d = Object.getOwnPropertyDescriptor(Map.prototype, "size"); return [typeof d.get, d.set, "value" in d, d.enumerable, d.configurable]; });
show("getter-shape ", () => { const g = getter(Map, "size"); return [typeof g, g.name, g.length, String(g)]; });
show("regexp-descr ", () => { const d = Object.getOwnPropertyDescriptor(RegExp.prototype, "source"); return [typeof d.get, d.get.name, d.enumerable]; });
show("symbol-descr ", () => { const d = Object.getOwnPropertyDescriptor(Symbol.prototype, "description"); return [typeof d.get, d.get.name]; });
// Borrowing the getter is the point of exposing it: it reads the slot off any
// receiver that carries one, and throws naming itself for one that does not.
show("borrow-ok    ", () => [getter(Map, "size").call(new Map([[1, 2]])), getter(Set, "size").call(new Set([1, 2, 3]))]);
show("borrow-buffer", () => getter(ArrayBuffer, "byteLength").call(new ArrayBuffer(8)));
show("borrow-view  ", () => [getter(DataView, "byteLength").call(new DataView(new ArrayBuffer(4), 1)), getter(DataView, "byteOffset").call(new DataView(new ArrayBuffer(4), 1))]);
show("borrow-symbol", () => getter(Symbol, "description").call(Symbol("d")));
// The brand check is not a chain walk: inheriting from the prototype is not
// carrying the slot.
show("created      ", () => { const o = Object.create(Map.prototype); try { return o.size; } catch (e) { return e.message; } });
show("wrong-plain  ", () => { try { return getter(Map, "size").call({}); } catch (e) { return e.message; } });
show("wrong-number ", () => { try { return getter(Map, "size").call(5); } catch (e) { return e.message; } });
show("wrong-array  ", () => { try { return getter(Map, "size").call([1, 2]); } catch (e) { return e.message; } });
show("wrong-null   ", () => { try { return getter(Map, "size").call(null); } catch (e) { return e.message; } });
show("cross-brand  ", () => { try { return getter(ArrayBuffer, "byteLength").call(new DataView(new ArrayBuffer(4))); } catch (e) { return e.message; } });
// …including the prototype itself, which renders `#<Map>` and not `[object Map]`.
show("on-prototype ", () => { try { return Map.prototype.size; } catch (e) { return e.message; } });
show("proto-recv   ", () => { try { return getter(Set, "size").call(Set.prototype); } catch (e) { return e.message; } });
// `RegExp.prototype` is the documented exception (22.2.6.x): it answers rather
// than throwing — `source` is `"(?:)"`, `flags` is `""`, every flag `undefined`.
show("regexp-proto ", () => [RegExp.prototype.source, RegExp.prototype.flags, RegExp.prototype.global, RegExp.prototype.dotAll]);
show("regexp-real  ", () => [/ab+/gi.source, /ab+/gi.flags, /ab+/gi.global, /ab+/gi.dotAll]);
show("regexp-wrong ", () => { try { return getter(RegExp, "global").call({}); } catch (e) { return e.message; } });
// `flags` alone is GENERIC (22.2.6.5): it reads the individual flag properties
// off whatever it is handed, so a plain object answers rather than throwing.
show("flags-generic", () => [getter(RegExp, "flags").call({}), getter(RegExp, "flags").call({ global: true, ignoreCase: true }), getter(RegExp, "flags").call({ hasIndices: true, sticky: true, unicode: true })]);
show("flags-nonobj ", () => { try { return getter(RegExp, "flags").call(5); } catch (e) { return e.message; } });
// `Symbol.prototype.description` words its brand failure its own way.
show("symbol-wrong ", () => { try { return getter(Symbol, "description").call({}); } catch (e) { return e.message; } });
// `Function.prototype.arguments`/`caller` are POISON PILLS: both halves throw
// for every receiver, and they are the only pair here with a setter at all.
show("poison-get   ", () => { try { return getter(Function, "arguments").call(function () {}); } catch (e) { return e.message; } });
show("poison-set   ", () => { const d = Object.getOwnPropertyDescriptor(Function.prototype, "caller"); return [typeof d.set, d.set.name]; });
show("poison-call  ", () => { const d = Object.getOwnPropertyDescriptor(Function.prototype, "caller"); try { return d.set.call(function () {}, 1); } catch (e) { return e.message; } });
// The instance side keeps working, and `in` still sees every one of them.
show("instances    ", () => [new Map([[1, 2]]).size, new Set([1]).size, Symbol("z").description, new ArrayBuffer(4).byteLength]);
show("in-operator  ", () => ["size" in new Map(), "source" in /a/, "description" in Symbol("x"), "byteLength" in new ArrayBuffer(1)]);
