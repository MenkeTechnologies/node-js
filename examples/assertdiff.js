// The structural `+ actual - expected` diff a failing assertion carries.
//
// Node renders both operands with util.inspect and prints a line diff of the
// two renderings; only when each side fits on ONE line does it fall back to
// `actual !== expected`, or to the stacked form with a caret under the first
// differing character.
const assert = require("assert");

function show(label, fn) {
  try {
    fn();
    console.log(label, "-> passed");
  } catch (e) {
    console.log("### " + label + "  (operator=" + e.operator + ", generated=" + e.generatedMessage + ")");
    console.log(e.message);
    console.log("<<<");
  }
}

show("one key differs", () => assert.deepStrictEqual({ a: 1, b: 2 }, { a: 1, b: 3 }));
show("extra key", () => assert.deepStrictEqual({ z: 1, a: 2 }, { z: 1, a: 2, c: 9 }));
show("longer array", () => assert.deepStrictEqual([1, 2], [1, 2, 3]));
show("every key differs", () => assert.deepStrictEqual({ a: 1, b: 2, c: 3 }, { a: 9, b: 8, c: 7 }));
show("nested", () => assert.deepStrictEqual({ x: { y: 1 } }, { x: { y: 2 } }));
show("sets", () => assert.deepStrictEqual(new Set([1, 2]), new Set([1, 3])));
show("maps", () => assert.deepStrictEqual(new Map([["a", 1]]), new Map([["a", 2]])));
show("buffers", () => assert.deepStrictEqual(Buffer.from([0xff, 0x00]), Buffer.from([0xff, 0x01])));
show("collapsed run", () => assert.deepStrictEqual(
  Array.from({ length: 30 }, (_, i) => i),
  Array.from({ length: 30 }, (_, i) => (i === 25 ? 999 : i))));
show("short strings", () => assert.deepStrictEqual("abc", "abd"));
show("short numbers", () => assert.strictEqual(1, 2));
show("zeros", () => assert.strictEqual(0, -0));
show("long strings", () => assert.strictEqual("hello world foo", "hello world bar"));
show("two objects", () => assert.strictEqual({ a: 1 }, { a: 2 }));
show("same structure", () => assert.strictEqual({ a: 1 }, { a: 1 }));
show("custom message", () => assert.deepStrictEqual({ a: 1 }, { a: 2 }, "boom"));
show("notDeepStrictEqual", () => assert.notDeepStrictEqual({ a: 1 }, { a: 1 }));
