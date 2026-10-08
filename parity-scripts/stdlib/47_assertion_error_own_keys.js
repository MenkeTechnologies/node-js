// An AssertionError's own property names, in V8's order: `stack`, `message`,
// `generatedMessage`, `name`, then the enumerable `code`/`actual`/`expected`/
// `operator`/`diff` — for every failing assertion form and a constructed one.
const assert = require("assert");
const cases = [
  () => assert.strictEqual(1, 2),
  () => assert.ok(false),
  () => assert.deepStrictEqual({ a: 1 }, { a: 2 }),
  () => assert(0, "m"),
  () => assert.fail("x"),
  () => assert.throws(() => {}),
  () => assert.equal(1, 2, "msg"),
  () => assert.notStrictEqual(1, 1),
  () => assert.match("a", /b/),
  () => { throw new assert.AssertionError({ actual: 1, expected: 2, operator: "==" }); },
];
for (const c of cases) {
  try {
    c();
  } catch (e) {
    console.log(Object.getOwnPropertyNames(e).join(), "|", Object.keys(e).join(), e.generatedMessage, e.operator);
  }
}
