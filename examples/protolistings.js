// An intrinsic prototype's own-key listings. The ones this host builds as
// NAMESPACE HANDLES already answered from the generated table; the ones it
// builds as REAL OBJECTS — `Symbol.prototype`, `String.prototype`, the error
// hierarchy — answered from their own property maps, which carry neither the
// right names nor V8's order. `Symbol.prototype` reported `toLocaleString` and
// omitted `description`; `String.prototype` omitted `length` and all nine
// Annex B HTML methods. Both representations now share one arm, so they cannot
// answer differently.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const names = (C) => Object.getOwnPropertyNames(C.prototype);
const syms = (C) => Object.getOwnPropertySymbols(C.prototype).map(String);

// The real-object prototypes, in V8's order — which is neither alphabetical nor
// the order a property map would produce.
show("symbol       ", () => [names(Symbol), syms(Symbol)]);
show("number       ", () => names(Number));
show("boolean      ", () => names(Boolean));
show("bigint       ", () => [names(BigInt), syms(BigInt)]);
show("string-count ", () => { const n = names(String); return [n.length, n[0], n.includes("anchor"), n.includes("trimLeft"), syms(String)]; });
show("string-annexb", () => ["big", "blink", "bold", "fixed", "fontcolor", "fontsize", "italics", "link", "small", "strike", "sub", "sup"].every((m) => names(String).includes(m)));
show("error        ", () => names(Error));
show("typeerror    ", () => [names(TypeError), names(RangeError), names(SyntaxError)]);
// The namespace-handle ones answer the same way, and the two agree about a
// prototype's accessors and its symbol members.
show("map-set      ", () => [names(Map), names(Set)]);
show("regexp       ", () => [names(RegExp), syms(RegExp)]);
show("promise      ", () => [names(Promise), syms(Promise)]);
show("array-syms   ", () => syms(Array));
show("date         ", () => [names(Date).length, names(Date)[0], syms(Date)]);
// A symbol-keyed member is never reported as a NAME, in either representation.
show("no-at-names  ", () => [names(Symbol), names(Map), names(RegExp), names(String)].every((ns) => ns.every((n) => !n.startsWith("@@"))));
// `Object.keys` still reports nothing: an ECMAScript prototype's members are
// all non-enumerable, whichever representation backs it.
show("enumerable   ", () => [Object.keys(Symbol.prototype), Object.keys(String.prototype), Object.keys(Map.prototype)]);
// …and the descriptors agree with the listings, for a method and an accessor
// on each kind of prototype.
show("descriptors  ", () => {
  const d = (C, k) => { const x = Object.getOwnPropertyDescriptor(C.prototype, k); return [typeof (x.value ?? x.get), x.writable ?? null, x.enumerable, x.configurable]; };
  return [d(Symbol, "description"), d(Symbol, "toString"), d(String, "charAt"), d(Map, "size"), d(Map, "get")];
});
