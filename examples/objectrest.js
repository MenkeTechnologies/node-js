// Object REST (`const { a, ...r } = o`) copied the property MAP, so it missed
// an ACCESSOR entirely — `const { ...r } = { get g() { … } }` produced an
// object with no `g` and never ran the getter — and it ran no Proxy traps at
// all. CopyDataProperties (7.3.25) reads each value through `[[Get]]`.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
show("getter       ", () => { let n = 0; const { ...r } = { get g() { n++; return 1; } }; return [r.g, n]; });
show("getter-desc  ", () => { const { ...r } = { get g() { return 1; } }; return Object.getOwnPropertyDescriptor(r, "g"); });
show("order        ", () => { const seen = []; const { ...r } = { get a() { seen.push("a"); return 1; }, get b() { seen.push("b"); return 2; } }; return [seen, Object.keys(r)]; });
// An EXCLUDED key's getter still runs — the binding needs its value.
show("excluded     ", () => { let n = 0; const { a, ...r } = { get a() { n++; return 1; }, b: 2 }; return [a, n, Object.keys(r)]; });
show("throwing     ", () => { const { ...r } = { get boom() { throw new TypeError("x"); } }; return r; });
// A SYMBOL key is copied; `Object.keys` deliberately omits it, so the key list
// cannot come from there alone.
show("symbol       ", () => { const s = Symbol("k"); const { ...r } = { [s]: 1, a: 2 }; return [Object.getOwnPropertySymbols(r).length, Object.keys(r)]; });
show("computed-excl", () => { const k = "a"; const { [k]: v, ...r } = { a: 1, b: 2 }; return [v, r]; });
// A non-enumerable own property and an inherited one are both omitted.
show("non-enumerable", () => { const { ...r } = Object.defineProperty({ a: 1 }, "h", { value: 2 }); return Object.keys(r); });
show("inherited    ", () => { const { ...r } = Object.create({ inherited: 1 }, { own: { value: 2, enumerable: true } }); return Object.keys(r); });

// Over a PROXY the keys and their enumerability come from the traps, and the
// per-key test and READ interleave.
show("proxy        ", () => {
  const log = [];
  const p = new Proxy({ a: 1, b: 2 }, {
    ownKeys(t) { log.push("ownKeys"); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor(t, k) { log.push("gopd:" + k); return Reflect.getOwnPropertyDescriptor(t, k); },
    get(t, k) { log.push("get:" + k); return t[k]; },
  });
  const { ...r } = p;
  return [r, log];
});

// Exotic sources, and the shapes that already worked.
show("array-source ", () => { const { ...r } = [1, 2]; return r; });
show("string-source", () => { const { ...r } = "ab"; return r; });
show("map-source   ", () => { const m = new Map([[1, 2]]); m.x = 3; const { ...r } = m; return r; });
show("class-inst   ", () => { class C { constructor() { this.a = 1; } get g() { return 2; } } const { ...r } = new C(); return Object.keys(r); });
show("frozen-source", () => { const { ...r } = Object.freeze({ a: 1 }); return [r, Object.isFrozen(r)]; });
show("nested       ", () => { const { a: { b, ...inner }, ...outer } = { a: { b: 1, c: 2 }, d: 3 }; return [b, inner, outer]; });
show("array-rest   ", () => { const [a, ...r] = [1, 2, 3]; return [a, r]; });
show("string-rest  ", () => { const [a, ...r] = "abc"; return [a, r]; });
show("param-rest   ", () => { function f(a, ...r) { return [a, r]; } return f(1, 2, 3); });

// Object SPREAD already read through getters; pinned so the two stay in step.
show("spread-getter", () => { let n = 0; const r = { ...{ get g() { n++; return 7; } } }; return [r.g, n, Object.getOwnPropertyDescriptor(r, "g").writable]; });
show("spread-order ", () => Object.keys({ ...{ b: 1, a: 2 }, c: 3, ...{ d: 4 } }));
show("spread-proto ", () => { const r = { ...JSON.parse('{"__proto__":{"x":1}}') }; return [r.x, Object.getPrototypeOf(r) === Object.prototype, Object.keys(r)]; });
show("spread-prims ", () => [{ ..."ab" }, { ...5 }, { ...null }, { ...undefined }, { ...true }]);
