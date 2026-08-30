// `assert`. Nothing in the corpus reached it, and its two comparison
// primitives were both wrong in ways that make a passing test suite prove
// nothing.
//
// Only the pass/fail OUTCOME is printed, not the AssertionError message —
// node builds a structured `+ actual / - expected` diff, which this does not.
const assert = require("assert");
const at = (f) => { try { f(); return "pass"; } catch (e) { return "throw:" + (e.code || e.constructor.name); } };

// `strictEqual` compares with `Object.is`, not `===`. That is the whole
// difference for two values, and it was backwards both ways: NaN failed to
// equal itself, and +0 compared equal to -0.
console.log("nan     ", at(() => assert.strictEqual(NaN, NaN)), at(() => assert.notStrictEqual(NaN, NaN)));
console.log("zero    ", at(() => assert.strictEqual(0, -0)), at(() => assert.notStrictEqual(0, -0)));
console.log("ordinary", at(() => assert.strictEqual(1, 1)), at(() => assert.strictEqual(1, "1")));

// `deepStrictEqual` requires the two to share a [[Prototype]]. That one check
// separates a null-prototype object from a plain one, an instance of one class
// from an instance of another, and a Uint8Array from an Int8Array — none of
// which were being distinguished, so all three compared EQUAL.
class Alpha {}
class Beta {}
console.log("proto   ", at(() => assert.deepStrictEqual(Object.create(null), {})));
console.log("classes ", at(() => assert.deepStrictEqual(new Alpha(), new Beta())), at(() => assert.deepStrictEqual(new Alpha(), new Alpha())));
console.log("typed   ", at(() => assert.deepStrictEqual(new Uint8Array([1]), new Int8Array([1]))), at(() => assert.deepStrictEqual(new Uint8Array([1]), new Uint8Array([1]))));
// The same Object.is rule applies to the values inside.
console.log("deep-nan", at(() => assert.deepStrictEqual([NaN], [NaN])), at(() => assert.deepStrictEqual([0], [-0])));

// A self-referential structure overflowed the stack and aborted the process.
// Comparing two of the same shape is what a test asserting on a linked or
// parent-pointing structure does.
const x = {}; x.self = x;
const y = {}; y.self = y;
console.log("cycle   ", at(() => assert.deepStrictEqual(x, y)));
const px = { name: "a" }; const py = { name: "a" };
px.peer = py; py.peer = px;
console.log("mutual  ", at(() => assert.deepStrictEqual(px, py)));
const differs = {}; differs.self = differs; differs.tag = 1;
console.log("cycle-ne", at(() => assert.deepStrictEqual(x, differs)));

// `require('assert/strict')` IS the strict namespace, so its `equal` and
// `deepEqual` are the strict comparisons. It was resolving to the plain module,
// so a strict-mode suite silently ran loose comparisons.
const strict = require("assert/strict");
console.log("strictns", at(() => strict.equal(1, "1")), at(() => strict.equal(1, 1)));
console.log("strictdp", at(() => strict.deepEqual({ a: 1 }, { a: "1" })), at(() => assert.strict.equal(1, "1")));
// The loose entry points stay loose.
console.log("loose   ", at(() => assert.equal(1, "1")), at(() => assert.deepEqual({ a: 1 }, { a: "1" })));

// The rest of the surface, unchanged.
console.log("shapes  ", at(() => assert.ok(1)), at(() => assert.ok(0)), at(() => assert.throws(() => { throw new TypeError("x"); }, TypeError)));
console.log("more    ", at(() => assert.match("abc", /b/)), at(() => assert.ifError(null)), typeof assert.AssertionError);
