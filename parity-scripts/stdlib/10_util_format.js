// util: format with %s %d %i %f %j %o %% specifiers.
const util = require("util");

console.log(util.format("%s = %d", "count", 42));
console.log(util.format("%s and %s", "foo", "bar"));
console.log(util.format("int %i float %f", 3.9, 3.14));
console.log(util.format("json %j", { a: 1, b: [2, 3] }));
console.log(util.format("100%% done"));
console.log(util.format("%s", "extra", "args", "appended"));
console.log(util.format("no specifiers", 1, 2, 3));
console.log(util.format("%d", "not-a-number"));
console.log(util.format("%s", 42n));
console.log(util.format("%o", { nested: { x: 1 } }));
