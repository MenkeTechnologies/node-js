// util.format directives that are NOT String()/inspect aliases: %s picks
// between them per value, %o is not %O, and three of them can throw.
const util = require("util");

class Custom { toString() { return "CUSTOM"; } }
class Plain {}

// %s inspects an object unless the SCRIPT gave it a toString.
for (const [label, v] of [
  ["plain", { a: 1 }], ["nested", { a: { b: { c: 1 } } }], ["array", [1, 2]],
  ["date", new Date(0)], ["regexp", /a/g], ["map", new Map([["a", 1]])],
  ["set", new Set([1])], ["empty map", new Map()], ["empty obj", {}],
  ["custom toString", new Custom()], ["no toString", new Plain()],
  ["null proto", Object.create(null)], ["boxed", new String("b")],
  ["symbol", Symbol("q")], ["bigint", 7n], ["number", 5],
  ["negative zero", -0], ["positive zero", 0], ["nan", NaN], ["infinity", Infinity],
  ["typed array", new Uint8Array([1, 2])], ["buffer", Buffer.from([104, 105])],
  ["array buffer", new ArrayBuffer(2)],
]) console.log(label, "=>", JSON.stringify(util.format("%s", v)));

// %o implies showHidden and depth 4; %O is the plain default-depth inspect.
console.log("%o array:", util.format("%o", [1, 2]));
console.log("%o regexp:", util.format("%o", /x/g));
console.log("%o deep:", JSON.stringify(util.format("%o", { a: { b: { c: { d: { e: 1 } } } } })));
console.log("%O deep:", JSON.stringify(util.format("%O", { a: { b: { c: { d: 1 } } } })));

// %d, %i and %f use three different conversions.
console.log(util.format("%d %i %f", 1.7, "3.9abc", "1.5x"));
console.log(util.format("%d %i", 10n, 10n));
console.log(util.format("%d %i %f", Symbol("s"), Symbol("s"), Symbol("s")));

// Throwing directives propagate; a circular structure does not.
const t = (label, fn) => {
  try { console.log(label, "=>", JSON.stringify(fn())); }
  catch (e) { console.log(label, "THREW", e.name + ":", e.message); }
};
t("%j bigint", () => util.format("%j", 1n));
t("%j nested bigint", () => util.format("%j", { a: 1n }));
t("%d symbol in array", () => util.format("%d", [Symbol("x")]));
t("%i symbol in array", () => util.format("%i", [Symbol("x")]));
t("%f symbol in array", () => util.format("%f", [Symbol("x")]));
const circ = {}; circ.self = circ;
t("%j circular", () => util.format("%j", circ));

// parseInt/parseFloat start from ToString(argument), which can throw and which
// honors a scripted toString.
t("parseInt array", () => parseInt(["12"]));
t("parseInt object", () => parseInt({ toString() { return "42"; } }));
t("parseInt symbol", () => parseInt(Symbol("x")));
t("parseInt symbol in array", () => parseInt([Symbol("x")]));
t("parseFloat object", () => parseFloat({ toString() { return "1.5"; } }));
t("parseInt nested", () => parseInt([[["7"]]]));
