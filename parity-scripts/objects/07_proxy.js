// Proxy: every trap reached through the OPERATOR that triggers it, the
// transparency of a trapless handler, revocation, and the shapes that classify
// by the target rather than the handler (typeof / isArray / brand).

// ── each trap, observed through its operator ────────────────────────────────
const log = [];
const target = { a: 1, b: 2 };
const p = new Proxy(target, {
  get(t, k) { log.push("get:" + String(k)); return t[k] * 10; },
  set(t, k, v) { log.push("set:" + String(k)); t[k] = v; return true; },
  has(t, k) { log.push("has:" + String(k)); return k === "ghost" || k in t; },
  deleteProperty(t, k) { log.push("del:" + String(k)); delete t[k]; return true; },
  ownKeys() { return ["a", "invented"]; },
  getOwnPropertyDescriptor(t, k) {
    return { value: k === "invented" ? 42 : t[k], enumerable: true, configurable: true, writable: true };
  },
});
console.log(p.a, p.b);
p.c = 3;
console.log("ghost" in p, "nope" in p, target.c);
console.log(delete p.b, target.b);
console.log(log.join("|"));
console.log(JSON.stringify(Object.keys(p)));
console.log(JSON.stringify(Object.getOwnPropertyNames(p)));
const inKeys = [];
for (const k in p) inKeys.push(k);
console.log(JSON.stringify(inKeys));

// ── a trapless handler is invisible ─────────────────────────────────────────
const plain = new Proxy({ x: 1, y: 2 }, {});
plain.z = 3;
console.log(plain.x, "y" in plain, JSON.stringify(Object.keys(plain)), JSON.stringify(plain));
console.log(JSON.stringify({ ...plain }), plain instanceof Object, plain.hasOwnProperty("x"));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(plain, "x")));

const arr = new Proxy([1, 2, 3], {});
console.log(arr.length, Array.isArray(arr), JSON.stringify([...arr]), JSON.stringify(arr.map((v) => v * 2)));
console.log(JSON.stringify(arr), Object.prototype.toString.call(arr));

// ── apply / construct / prototype ───────────────────────────────────────────
const pf = new Proxy(function (a, b) { return a + b; }, { apply(t, self, args) { return t(...args) * 2; } });
console.log(typeof pf, pf(1, 2), pf.name, pf.length, pf.call(null, 4, 5));

class C { constructor(x) { this.x = x; } m() { return "m" + this.x; } }
const pc = new Proxy(C, { construct(t, args) { return new t(args[0] + 100); } });
const made = new pc(1);
console.log(made.x, made.m(), made instanceof C);

const pp = new Proxy({}, { getPrototypeOf() { return Array.prototype; } });
console.log(Object.getPrototypeOf(pp) === Array.prototype, pp instanceof Array);

// ── a proxy as prototype: the trap's receiver is the CHILD ──────────────────
const base = { get who() { return this.name; } };
const proto = new Proxy(base, { get(t, k, r) { return Reflect.get(t, k, r); } });
console.log(Object.create(proto, { name: { value: "child" } }).who);

const methodProto = new Proxy({}, { get(t, k) { return k === "greet" ? () => "hi" : undefined; } });
console.log(Object.create(methodProto).greet());

class B { constructor() { this.b = 1; } n() { return "n"; } }
class D extends new Proxy(B, {}) { constructor() { super(); this.d = 2; } }
const d = new D();
console.log(d.b, d.d, d.n(), d instanceof B, d instanceof D);

// ── a get trap that lies about length is honored by iteration ───────────────
const short = new Proxy([1, 2, 3], { get: (t, k) => (k === "length" ? 2 : t[k]) });
console.log(short.length, JSON.stringify([...short]), JSON.stringify(short));

// ── symbol keys arrive as symbols ───────────────────────────────────────────
const S = Symbol("tag");
const seen = [];
const ps = new Proxy({}, {
  get(t, k) { seen.push("get:" + typeof k); return t[k]; },
  set(t, k, v) { seen.push("set:" + typeof k); t[k] = v; return true; },
});
ps[S] = 1;
void ps[S];
void ps.plain;
console.log(seen.join(","));
const withSym = new Proxy({ [S]: 1, k: 2 }, {});
console.log(Object.getOwnPropertySymbols(withSym).length, Reflect.ownKeys(withSym).map(String).join("|"));
console.log(Object.prototype.toString.call(new Proxy({}, { get: (t, k) => (k === Symbol.toStringTag ? "Zed" : undefined) })));

// ── revocation ──────────────────────────────────────────────────────────────
const { proxy, revoke } = Proxy.revocable({ a: 1 }, {});
console.log(proxy.a);
revoke();
revoke();
console.log(typeof proxy);
for (const [label, f] of [
  ["get", () => proxy.a],
  ["has", () => "a" in proxy],
  ["ownKeys", () => Object.keys(proxy)],
  ["callProxy", () => Proxy({}, {})],
  ["badTarget", () => new Proxy(1, {})],
  ["badTrap", () => new Proxy({}, { get: 1 }).a],
]) {
  try { f(); console.log(label, "NO-THROW"); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
}
