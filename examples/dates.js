// A Date carries its time value in an internal slot, so an inspector that only
// walks enumerable properties renders every Date ever logged as `{}`.
const util = require("util");
const d = new Date(1);

console.log(d);
console.log([d]);
console.log({ d });
console.log(new Map([["k", d]]));
console.log(new Set([d]));
console.log(util.inspect(d, { depth: 0 }));
console.log(util.inspect({ a: { b: { c: d } } }));
console.log(new Date(0), new Date(86400000), new Date(-1));
console.log(new Date(NaN));

const tagged = new Date(0);
tagged.note = "x";
console.log(tagged);

console.log(util.format("%s", d));
try {
  require("assert").deepStrictEqual({ when: new Date(0) }, { when: new Date(1) });
} catch (e) {
  console.log(e.message);
}
