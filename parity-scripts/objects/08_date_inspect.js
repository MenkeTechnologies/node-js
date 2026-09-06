// A Date's time value lives in an internal slot, not an enumerable property, so
// it is only visible if the inspector knows the brand — otherwise every Date
// logged anywhere renders as an empty object.
const util = require("util");
const d = new Date(1);

console.log(d);
console.log([d]);
console.log({ d });
console.log(new Map([["k", d]]));
console.log(new Set([d]));
console.log(util.inspect(d));
console.log(util.inspect(d, { depth: 0 }));
console.log(util.inspect({ a: { b: { c: d } } }));
console.log(new Date(0), new Date(86400000), new Date(-1));
console.log(new Date(NaN));
console.log(util.inspect([new Date(NaN)]));

// Own properties render after the date itself.
const tagged = new Date(0);
tagged.note = "x";
console.log(tagged);

// %s and the assert renderer both reach the same string.
console.log(util.format("%s", d));
try {
  require("assert").deepStrictEqual({ when: new Date(0) }, { when: new Date(1) });
} catch (e) {
  console.log(e.message);
}
