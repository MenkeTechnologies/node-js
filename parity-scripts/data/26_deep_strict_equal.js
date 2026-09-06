// util.isDeepStrictEqual across every heap kind. The comparison must reach each
// brand's INTERNAL state: a Date's time value, a byte view's bytes, a Map's
// entries — none of which are enumerable properties, so a comparison that looks
// only at those reports every pair of them as equal.
const util = require("util");

const pairs = [
  ["string ne", "abc", "abd"],
  ["string eq", "abc", "abc"],
  ["date ne", new Date(0), new Date(1)],
  ["date eq", new Date(0), new Date(0)],
  ["regexp ne", /a/g, /b/g],
  ["regexp eq", /a/g, /a/g],
  ["regexp flags", /a/g, /a/i],
  ["u8 ne", new Uint8Array([1, 2]), new Uint8Array([1, 3])],
  ["u8 eq", new Uint8Array([1, 2]), new Uint8Array([1, 2])],
  ["buffer ne", Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]), Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x81])],
  ["buffer eq", Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]), Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80])],
  ["error ne", new Error("a"), new Error("b")],
  ["bigint ne", 1n, 2n],
  ["bigint eq", 1n, 1n],
  ["boxed ne", new String("a"), new String("b")],
  ["number obj ne", new Number(1), new Number(2)],
  ["set ne", new Set([1, 2]), new Set([1, 3])],
  ["set eq", new Set([1, 2]), new Set([1, 2])],
  ["set reordered", new Set([1, 2]), new Set([2, 1])],
  ["set size", new Set([1]), new Set([1, 2])],
  ["set nested", new Set([{ a: 1 }]), new Set([{ a: 1 }])],
  ["map ne", new Map([["a", 1]]), new Map([["a", 2]])],
  ["map eq", new Map([["a", 1]]), new Map([["a", 1]])],
  ["map reordered", new Map([["a", 1], ["b", 2]]), new Map([["b", 2], ["a", 1]])],
  ["map key ne", new Map([["a", 1]]), new Map([["b", 1]])],
  ["nested set", { s: new Set([1]) }, { s: new Set([2]) }],
  ["nested string", { s: "aa" }, { s: "ab" }],
  ["nested date", [new Date(0)], [new Date(1)]],
  ["array ne", [1, 2], [1, 3]],
  ["object eq", { a: 1 }, { a: 1 }],
  ["typed kind", new Uint8Array([1]), new Int8Array([1])],
];
for (const [label, a, b] of pairs) console.log(label, "=>", util.isDeepStrictEqual(a, b));

// The same relation drives assert, which must THROW for each unequal pair.
let threw = 0;
for (const [, a, b] of pairs) {
  try { require("assert").deepStrictEqual(a, b); } catch { threw++; }
}
console.log("threw:", threw);
