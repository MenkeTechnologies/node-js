// util.format's directives are not interchangeable: %s chooses between String()
// and inspect per value, %o is not %O, and three of them can throw.
const util = require("util");

class Custom { toString() { return "CUSTOM"; } }
class Plain {}

for (const [label, v] of [
  ["plain", { a: 1 }], ["nested", { a: { b: { c: 1 } } }], ["array", [1, 2]],
  ["date", new Date(0)], ["regexp", /a/g], ["map", new Map([["a", 1]])],
  ["set", new Set([1])], ["custom toString", new Custom()], ["no toString", new Plain()],
  ["null proto", Object.create(null)], ["boxed", new String("b")],
  ["bigint", 7n], ["negative zero", -0], ["nan", NaN],
  ["typed array", new Uint8Array([1, 2])], ["buffer", Buffer.from([104, 105])],
  ["array buffer", new ArrayBuffer(2)],
]) console.log(label, "=>", JSON.stringify(util.format("%s", v)));

console.log("%o array:", util.format("%o", [1, 2]));
console.log("%o regexp:", util.format("%o", /x/g));
console.log("%o depth:", JSON.stringify(util.format("%o", { a: { b: { c: { d: { e: 1 } } } } })));
console.log("%O depth:", JSON.stringify(util.format("%O", { a: { b: { c: { d: 1 } } } })));
console.log(util.format("%d %i %f", 1.7, "3.9abc", "1.5x"));

const t = (label, fn) => {
  try { console.log(label, "=>", JSON.stringify(fn())); }
  catch (e) { console.log(label, "THREW", e.name + ":", e.message); }
};
t("%j bigint", () => util.format("%j", 1n));
t("%d symbol in array", () => util.format("%d", [Symbol("x")]));
const circ = {}; circ.self = circ;
t("%j circular", () => util.format("%j", circ));
t("parseInt array", () => parseInt(["12"]));
t("parseInt object", () => parseInt({ toString() { return "42"; } }));
t("parseInt symbol in array", () => parseInt([Symbol("x")]));
t("parseFloat object", () => parseFloat({ toString() { return "1.5"; } }));
