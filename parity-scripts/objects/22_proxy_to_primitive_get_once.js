// OrdinaryToPrimitive on a Proxy: each candidate method is read once through
// the `get` trap and then called, never read a second time.
const mk = (t) => {
  const log = [];
  const p = new Proxy(t, {
    get(t, k, r) {
      if (typeof k !== "symbol") log.push("get:" + String(k));
      return Reflect.get(t, k, r);
    },
  });
  return [p, log];
};
const targets = [
  ["obj", {}],
  ["arr", [1, 2]],
  ["fn", function f() {}],
  ["custom", { toString() { return "T"; }, valueOf() { return 7; } }],
];
for (const [label, t] of targets) {
  for (const [op, run] of [["String", (p) => String(p)], ["tmpl", (p) => `${p}`], ["plus", (p) => p + 1], ["num", (p) => +p]]) {
    const [p, log] = mk(t);
    let s;
    try { s = String(run(p)).slice(0, 24); } catch (e) { s = e.constructor.name; }
    console.log(label, op, s, log.join(" "));
  }
}
