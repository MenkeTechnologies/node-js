// assert: strictEqual / deepStrictEqual / throws / ok — all passing.
const assert = require("assert");

assert.strictEqual(1 + 1, 2);
assert.strictEqual("foo" + "bar", "foobar");
assert.notStrictEqual(1, "1");

assert.deepStrictEqual({ a: 1, b: [2, 3] }, { a: 1, b: [2, 3] });
assert.deepStrictEqual([1, [2, [3]]], [1, [2, [3]]]);
assert.notDeepStrictEqual({ a: 1 }, { a: 2 });

assert.ok(true);
assert.ok(1);

assert.throws(() => {
  throw new TypeError("boom");
}, TypeError);

assert.throws(
  () => {
    throw new Error("specific message");
  },
  { message: "specific message" },
);

assert.doesNotThrow(() => {
  return 42;
});

console.log("all assertions passed");
