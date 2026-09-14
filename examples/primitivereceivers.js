// A string, a symbol and a bigint are PRIMITIVES that ride as heap handles in
// this host, so `matches!(v, Value::Obj(_))` is not "is this an object". Three
// entry points were gated on that shape and answered for a primitive as though
// it were an object; the semantic predicates `is_primitive`/`is_object_like`
// are what `ToObject` and `typeof` already share.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const sym = Symbol("s"), big = 1n, str = "ab";

// `Reflect.set` with a primitive RECEIVER reports false (10.1.9.2 step 3.b) —
// it does not throw. Only a number and a boolean took that path.
show("reflect-set  ", () => [Reflect.set({}, "x", 1, str), Reflect.set({}, "x", 1, sym), Reflect.set({}, "x", 1, big), Reflect.set({}, "x", 1, 5), Reflect.set({}, "x", 1, true)]);
show("reflect-null ", () => Reflect.set({}, "x", 1, null));
// …and none of that disturbs an ordinary one.
show("reflect-ok   ", () => { const o = {}; return [Reflect.set(o, "x", 1), o.x]; });
show("reflect-recv ", () => { const o = {}, r = {}; return [Reflect.set(o, "x", 1, r), o.x, r.x]; });
// `instanceof` has TWO messages for a bad right-hand side, and which one it
// uses turns on exactly this distinction.
show("instanceof-s ", () => { try { return ({}) instanceof str; } catch (e) { return e.message; } });
show("instanceof-y ", () => { try { return ({}) instanceof sym; } catch (e) { return e.message; } });
show("instanceof-b ", () => { try { return ({}) instanceof big; } catch (e) { return e.message; } });
show("instanceof-n ", () => { try { return ({}) instanceof 5; } catch (e) { return e.message; } });
show("instanceof-o ", () => { try { return ({}) instanceof {}; } catch (e) { return e.message; } });
show("instanceof-ok", () => [({}) instanceof Object, str instanceof Object]);
// The integrity predicates answer for a primitive WITHOUT coercing it: not
// extensible, and vacuously frozen and sealed. All three were the opposite.
show("integrity-str", () => [Object.isExtensible(str), Object.isFrozen(str), Object.isSealed(str)]);
show("integrity-sym", () => [Object.isExtensible(sym), Object.isFrozen(sym), Object.isSealed(sym)]);
show("integrity-big", () => [Object.isExtensible(big), Object.isFrozen(big), Object.isSealed(big)]);
show("integrity-num", () => [Object.isExtensible(5), Object.isFrozen(5), Object.isSealed(5)]);
// A BOXED primitive is an object and answers as one.
show("boxed        ", () => { const b = Object("ab"); return [Object.isExtensible(b), Object.isFrozen(b), Object.isSealed(b)]; });
show("plain-object ", () => { const o = {}; return [Object.isExtensible(o), Object.isFrozen(o), Object.isSealed(o)]; });
show("frozen-object", () => { const o = Object.freeze({}); return [Object.isExtensible(o), Object.isFrozen(o), Object.isSealed(o)]; });
// The mutating forms hand a primitive straight back.
show("seal-freeze  ", () => [Object.seal(str) === str, Object.freeze(sym) === sym, Object.preventExtensions(big) === big]);
