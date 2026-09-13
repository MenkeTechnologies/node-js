// A PROTOCOL method — `toJSON`, `Symbol.toPrimitive`, `Symbol.hasInstance`,
// `Symbol.asyncIterator`, an error's `name`/`message` — is looked up with
// `[[Get]]`, so a Proxy supplies one through its `get` trap. Every one of these
// walked the property MAP instead, never asked the handler, and fell back to
// the default behaviour as though the proxy had no such method.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
// Answer `name` from the trap and nothing else from the target.
const supplying = (name, value) => new Proxy({}, { get: (t, k) => (k === name ? value : t[k]) });

show("toJSON       ", () => JSON.stringify(new Proxy({ a: 1 }, { get: (t, k) => (k === "toJSON" ? () => "REPLACED" : t[k]) })));
show("toJSON-nested", () => JSON.stringify({ v: supplying("toJSON", () => 42) }));
show("toPrimitive  ", () => { const p = new Proxy({}, { get: (t, k) => (k === Symbol.toPrimitive ? () => "PRIM" : undefined) }); return `${p}`; });
show("hasInstance  ", () => { const p = new Proxy(function () {}, { get: (t, k) => (k === Symbol.hasInstance ? () => true : t[k]) }); return ({}) instanceof p; });
show("toStringTag  ", () => { const p = new Proxy({}, { get: (t, k) => (k === Symbol.toStringTag ? "TAG" : undefined) }); return Object.prototype.toString.call(p); });
show("iterator     ", () => { const p = new Proxy({}, { get: (t, k) => (k === Symbol.iterator ? function* () { yield 5; } : undefined) }); return [...p]; });

show("toString     ", () => String(supplying("toString", () => "STR")));
show("valueOf      ", () => supplying("valueOf", () => 9) * 2);
show("error-name   ", () => String(new Proxy(new Error("m"), { get: (t, k) => (k === "name" ? "PROXYERR" : t[k]) })));
// These two settle asynchronously, so their RESOLVED values are printed rather
// than the promise — a pending promise stringifies as `{}` and would assert
// nothing.
(async () => {
  const thenable = supplying("then", (res) => res("THENABLE"));
  console.log("then         ", JSON.stringify(await Promise.resolve(thenable)));
  const p = new Proxy({}, { get: (t, k) => (k === Symbol.asyncIterator ? async function* () { yield 3; } : undefined) });
  const out = [];
  for await (const v of p) out.push(v);
  console.log("asyncIterator", JSON.stringify(out));
})();

// `JSON.stringify` reads `toJSON` FIRST, which the trap log shows.
show("trap-order   ", () => {
  const log = [];
  const p = new Proxy({ a: 1 }, new Proxy({}, {
    get: (_, trap) => (...x) => { log.push(trap); return Reflect[trap](...x); },
  }));
  JSON.stringify(p);
  return log;
});
// A proxy with no such method still behaves as a plain object.
show("plain        ", () => [JSON.stringify(new Proxy({ a: 1 }, {})), JSON.stringify(new Proxy([1, 2], {}))]);

// The same protocols on an ORDINARY object, pinned so the proxy branch cannot
// drift from them.
show("own-toJSON   ", () => JSON.stringify({ toJSON: () => "P" }));
show("own-primitive", () => `${{ [Symbol.toPrimitive]: () => "OWN" }}`);
show("own-instance ", () => { class C { static [Symbol.hasInstance]() { return true; } } return ({}) instanceof C; });
show("own-error    ", () => String(Object.assign(new Error("m"), { name: "OWNERR" })));

// Every other copy operation already read through `[[Get]]`; pinned together so
// a future change cannot regress one of them alone.
const counting = () => { let n = 0; return { src: { get g() { n++; return 7; }, plain: 1 }, count: () => n }; };
for (const [name, op] of [["assign", (s) => Object.assign({}, s)], ["values", (s) => Object.values(s)],
  ["entries", (s) => Object.entries(s)], ["stringify", (s) => JSON.parse(JSON.stringify(s))],
  ["spread", (s) => ({ ...s })], ["rest", (s) => { const { ...r } = s; return r; }]]) {
  show(("copies-" + name).padEnd(14), () => { const { src, count } = counting(); return [op(src), count()]; });
}
