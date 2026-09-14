// `PROTO_MEMBERS` is generated from the prototypes' `getOwnPropertyNames`, so
// the SYMBOL-keyed members were absent from it entirely: `Symbol.iterator in
// []` answered false while `[][Symbol.iterator]` read back a function, and
// `Object.getOwnPropertySymbols(Array.prototype)` was `[]`. `gen-arity` now
// emits them under this frontend's internal `@@name` spelling.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// `in` and the read agree, which is the invariant that was broken.
show("array        ", () => [Symbol.iterator in [], Symbol.unscopables in [], typeof [][Symbol.iterator]]);
show("map-set      ", () => [Symbol.iterator in new Map(), Symbol.toStringTag in new Map(), Symbol.iterator in new Set()]);
show("regexp       ", () => [Symbol.match in /a/, Symbol.replace in /a/, Symbol.split in /a/, Symbol.search in /a/, Symbol.matchAll in /a/]);
show("string-box   ", () => [Symbol.iterator in Object("a"), typeof Object("a")[Symbol.iterator]]);
show("function     ", () => [Symbol.hasInstance in function () {}, typeof Function.prototype[Symbol.hasInstance]]);
show("date         ", () => [Symbol.toPrimitive in new Date(), typeof Date.prototype[Symbol.toPrimitive]]);
show("promise      ", () => [Symbol.toStringTag in Promise.resolve(), Promise.prototype[Symbol.toStringTag]]);
show("typed-array  ", () => { const ta = new Uint8Array(1); return [Symbol.iterator in ta, Symbol.toStringTag in ta]; });
// The two listings split by key kind: `getOwnPropertyNames` is strings only.
show("symbols-array", () => Object.getOwnPropertySymbols(Array.prototype).map(String));
show("symbols-regex", () => Object.getOwnPropertySymbols(RegExp.prototype).map(String));
show("symbols-map  ", () => Object.getOwnPropertySymbols(Map.prototype).map(String));
show("names-split  ", () => [Object.getOwnPropertyNames(Array.prototype).filter((n) => n.startsWith("@@")), Object.getOwnPropertyNames(Array.prototype).includes("join")]);
show("descriptor   ", () => { const d = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator); return d && [typeof d.value, d.writable, d.enumerable, d.configurable]; });
// Each one is callable off the prototype with an explicit receiver — the form a
// library uses to borrow an iterator. `%TypedArray%.prototype[Symbol.iterator]`
// IS `values` (23.2.3.35) and dispatches as it, which it did not before.
show("borrow-array ", () => [...Array.prototype[Symbol.iterator].call([1, 2])]);
show("borrow-typed ", () => [...Uint8Array.prototype[Symbol.iterator].call(new Uint8Array([3, 4]))]);
show("borrow-buffer", () => [...Buffer.from([5, 6])[Symbol.iterator]()]);
// …and a detached view names the method it ALIASES, not the internal spelling.
show("detached     ", () => { const ab = new ArrayBuffer(2); const ta = new Uint8Array(ab); ab.transfer(); try { return [...ta]; } catch (e) { return e.message; } });
