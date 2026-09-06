// assert: the structural `+ actual - expected` diff a failing comparison
// carries, and the shapes that take the one-line or stacked form instead.
const assert = require("assert");

function show(label, fn) {
  try {
    fn();
    console.log(label, "-> passed");
  } catch (e) {
    console.log("### " + label + "  (" + e.name + ", operator=" + e.operator + ", generated=" + e.generatedMessage + ")");
    console.log(e.message);
    console.log("<<<");
  }
}

show("obj one key", () => assert.deepStrictEqual({ a: 1, b: 2 }, { a: 1, b: 3 }));
show("sorted keys + extra", () => assert.deepStrictEqual({ z: 1, a: 2 }, { z: 1, a: 2, c: 9 }));
show("array longer", () => assert.deepStrictEqual([1, 2], [1, 2, 3]));
show("all differ", () => assert.deepStrictEqual({ a: 1, b: 2, c: 3 }, { a: 9, b: 8, c: 7 }));
show("nested", () => assert.deepStrictEqual({ x: { y: 1 } }, { x: { y: 2 } }));
show("reordered array", () => assert.deepStrictEqual([1, 2, 3], [3, 2, 1]));
show("sets", () => assert.deepStrictEqual(new Set([1, 2]), new Set([1, 3])));
show("maps", () => assert.deepStrictEqual(new Map([["a", 1]]), new Map([["a", 2]])));
show("buffers", () => assert.deepStrictEqual(Buffer.from([0xff, 0x00]), Buffer.from([0xff, 0x01])));
// A long identical run collapses, and the header then says so.
show("skipped lines", () => assert.deepStrictEqual(
  Array.from({ length: 30 }, (_, i) => i),
  Array.from({ length: 30 }, (_, i) => (i === 25 ? 999 : i))));

// Short primitives take the one-line form, longer ones stack with a caret.
show("short strings", () => assert.deepStrictEqual("abc", "abd"));
show("short numbers", () => assert.strictEqual(1, 2));
show("mixed primitive", () => assert.deepStrictEqual(1, "1"));
show("zeros", () => assert.strictEqual(0, -0));
show("long strings", () => assert.strictEqual("hello world foo", "hello world bar"));
show("null vs undefined", () => assert.strictEqual(null, undefined));
show("prim vs object", () => assert.strictEqual(1, { a: 1 }));

// Two distinct objects under strictEqual: reference-equality wording, and the
// "same structure" wording when they render identically.
show("strictEqual objects", () => assert.strictEqual({ a: 1 }, { a: 2 }));
show("same structure", () => assert.strictEqual({ a: 1 }, { a: 1 }));

// A custom message replaces the heading but keeps the diff.
show("custom message", () => assert.deepStrictEqual({ a: 1 }, { a: 2 }, "boom"));
show("custom strictEqual", () => assert.strictEqual(1, 2, "boom"));

// The comparisons that echo one operand use assert's expanded rendering too.
show("notDeepStrictEqual", () => assert.notDeepStrictEqual({ a: 1 }, { a: 1 }));
show("deepEqual loose", () => assert.deepEqual({ a: 1 }, { a: 2 }));
