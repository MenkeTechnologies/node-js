// Destructuring, defaults, rest/spread, and optional chaining.
const { a, b: renamed, missing = "dflt", ...others } = { a: 1, b: 2, c: 3, d: 4 };
console.log(a, renamed, missing, others);
const [x, , z = 99, ...tail] = [10, 20, undefined, 40, 50];
console.log(x, z, tail);
const { deep: { inner } } = { deep: { inner: "found" } };
console.log(inner);
let p = 1, q = 2;
[p, q] = [q, p];
console.log(p, q);
const key = "dyn";
const { [key]: viaComputed } = { dyn: "computed" };
console.log(viaComputed);
function f({ n = 5, m } = {}, ...rest) { return [n, m, rest]; }
console.log(f(), f({ n: 1, m: 2 }, 3, 4));
const obj = { nested: { arr: [1, 2] } };
console.log(obj?.nested?.arr?.[1], obj?.nope?.deep, obj.nope?.().x);
console.log(null ?? "fallback", 0 ?? "no", "" || "empty-is-falsy", 0 || "zero");
let lv = null; lv ??= "assigned"; console.log(lv);
let lv2 = 5; lv2 ||= 9; lv2 &&= 7; console.log(lv2);
console.log({ ...{ s: 1 }, ...{ t: 2 } }, [...[1, 2], ...[3]]);
