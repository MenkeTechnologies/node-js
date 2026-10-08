// Array.prototype.concat: IsArray sees through a proxy, and a spread
// array-like asks HasProperty per index — a missing index stays a hole.
const log = [];
const tr = (n) => (...a) => {
  const k = a[1];
  if (typeof k !== "symbol") log.push(n + ":" + String(k));
  return Reflect[n](...a);
};
const p = new Proxy([1, , 3], { get: tr("get"), has: tr("has") });
const r = [0].concat(p);
console.log(r, r.length, 2 in r, log.join(" "));
const like = { length: 3, 0: "a", 2: "c", [Symbol.isConcatSpreadable]: true };
const r2 = [].concat(like, ["z"]);
console.log(r2, 1 in r2, r2.length);
const getterLike = { length: 2, get 0() { throw new Error("boom"); }, [Symbol.isConcatSpreadable]: true };
try { [].concat(getterLike); } catch (e) { console.log("threw", e.message); }
console.log([].concat(new Proxy({ length: 1, 0: "x" }, {})));
