// 13.15.2: assignment evaluates its TARGET reference — the object, then the
// property key — before the right-hand side. Every step below is observable
// only because each sub-expression records itself.
const log = [];
const t = (x) => { log.push(x); return x; };
const take = () => { const s = log.join(","); log.length = 0; return s; };

const o = {};
o[t("key")] = t("val");
console.log("computed member:", take());

const a = [];
a[t(0)] = t("v");
console.log("index:", take());

// The object expression comes first of all three.
const box = { inner: {} };
(t(box)).inner[t("k")] = t(1);
console.log("object then key then value:", take());

// A key with a side effect must not be re-evaluated by the store.
let i = 0;
const arr = [10, 20, 30];
arr[i++] = arr[i];
console.log("i =", i, "arr =", arr.join(","));

// The assignment expression still evaluates to the assigned value, and a
// chained assignment threads it through.
const p = {};
console.log("value of assignment:", (p.x = 7));
let q1, q2;
const r = {};
q1 = q2 = r.z = 5;
console.log("chained:", q1, q2, r.z);

// Setters see the assigned value, and the expression result is that value.
const s = { set v(x) { log.push("set:" + x); } };
console.log("setter result:", (s.v = t(9)), "order:", take());

// Nested/computed keys evaluate left to right.
const deep = { m: {} };
deep[t("m")][t("n")] = t("z");
console.log("nested computed:", take(), JSON.stringify(deep));
