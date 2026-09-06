// Structural equality has to reach each brand's INTERNAL state: a Date's time
// value, a byte view's bytes, a Map's entries. None of those are enumerable
// properties, so a comparison that inspects only properties calls every pair of
// them equal — and an assertion that never fails is worse than no assertion.
const util = require("util");
const assert = require("assert");

const pairs = [
  ["string ne", "abc", "abd"],
  ["date ne", new Date(0), new Date(1)],
  ["date eq", new Date(0), new Date(0)],
  ["regexp ne", /a/g, /b/g],
  ["regexp eq", /a/g, /a/g],
  ["regexp flags", /a/g, /a/i],
  ["u8 ne", new Uint8Array([1, 2]), new Uint8Array([1, 3])],
  ["buffer ne", Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]), Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x81])],
  ["buffer eq", Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]), Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80])],
  ["error ne", new Error("a"), new Error("b")],
  ["bigint ne", 1n, 2n],
  ["boxed ne", new String("a"), new String("b")],
  ["set ne", new Set([1, 2]), new Set([1, 3])],
  ["set reordered", new Set([1, 2]), new Set([2, 1])],
  ["set size", new Set([1]), new Set([1, 2])],
  ["map ne", new Map([["a", 1]]), new Map([["a", 2]])],
  ["map reordered", new Map([["a", 1], ["b", 2]]), new Map([["b", 2], ["a", 1]])],
  ["nested set", { s: new Set([1]) }, { s: new Set([2]) }],
  ["nested date", [new Date(0)], [new Date(1)]],
  ["typed kind", new Uint8Array([1]), new Int8Array([1])],
];
for (const [label, a, b] of pairs) console.log(label, "=>", util.isDeepStrictEqual(a, b));

let threw = 0;
for (const [, a, b] of pairs) {
  try { assert.deepStrictEqual(a, b); } catch { threw++; }
}
console.log("threw:", threw);
