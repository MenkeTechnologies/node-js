// Trap order on a Proxy: spread / Object.assign / Object.entries / values ask
// one key's descriptor and then read it before moving on; Object.keys never
// reads; JSON.stringify takes every descriptor first, and serializes an
// array-backed proxy by length + index reads alone.
const mk = (t) => {
  const log = [];
  const tr = (n) => (...a) => {
    const k = a[1];
    if (typeof k !== "symbol") log.push(n + ":" + String(k));
    return Reflect[n](...a);
  };
  const H = {
    get: tr("get"),
    has: tr("has"),
    getOwnPropertyDescriptor: tr("getOwnPropertyDescriptor"),
    ownKeys: (t) => { log.push("ownKeys"); return Reflect.ownKeys(t); },
  };
  return [new Proxy(t, H), log];
};
const ops = [
  ["spread", (p) => ({ ...p })],
  ["entries", Object.entries],
  ["values", Object.values],
  ["keys", Object.keys],
  ["assign", (p) => Object.assign({}, p)],
  ["json", JSON.stringify],
  ["forin", (p) => { const r = []; for (const k in p) r.push(k); return r; }],
];
for (const t of [() => ({ a: 1, b: 2 }), () => [1, 2], () => Object.defineProperty({ x: 1 }, "h", { value: 2 })]) {
  for (const [op, f] of ops) {
    const [p, log] = mk(t());
    let r;
    try { r = JSON.stringify(f(p)); } catch (e) { r = e.constructor.name; }
    console.log(op, r, log.join(" "));
  }
}
