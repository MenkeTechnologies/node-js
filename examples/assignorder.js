// 13.15.2: an assignment evaluates its TARGET reference — the object, then the
// property key — before it evaluates the right-hand side. Only a side effect in
// each sub-expression can tell the orders apart.
const log = [];
const t = (x) => { log.push(x); return x; };
const take = () => { const s = log.join(","); log.length = 0; return s; };

const o = {};
o[t("key")] = t("val");
console.log("computed member:", take());

const a = [];
a[t(0)] = t("v");
console.log("index:", take());

const box = { inner: {} };
(t(box)).inner[t("k")] = t(1);
console.log("object, key, value:", take());

let i = 0;
const arr = [10, 20, 30];
arr[i++] = arr[i];
console.log("i =", i, "arr =", arr.join(","));

const p = {};
console.log("value of assignment:", (p.x = 7));
let q1, q2;
const r = {};
q1 = q2 = r.z = 5;
console.log("chained:", q1, q2, r.z);

const s = { set v(x) { log.push("set:" + x); } };
console.log("setter result:", (s.v = t(9)), "order:", take());

const deep = { m: {} };
deep[t("m")][t("n")] = t("z");
console.log("nested computed:", take(), JSON.stringify(deep));
