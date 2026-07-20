// util: inspect of nested objects/arrays + types.isX predicates.
const util = require("util");

console.log(util.inspect({ a: 1, b: { c: 2, d: [3, 4] } }));
console.log(util.inspect([1, "two", true, null, undefined]));
console.log(util.inspect({ nested: { deep: { deeper: 1 } } }, { depth: 1 }));
console.log(util.inspect(new Map([["k", "v"]])));
console.log(util.inspect(new Set([1, 2, 3])));
console.log(util.inspect("a string"));
console.log(util.inspect({ fn: function named() {} }));

console.log(util.types.isDate(new Date()));
console.log(util.types.isRegExp(/abc/));
console.log(util.types.isMap(new Map()));
console.log(util.types.isSet(new Set()));
console.log(util.types.isPromise(Promise.resolve()));
console.log(util.types.isArrayBuffer(new ArrayBuffer(8)));
console.log(util.types.isNativeError(new TypeError("x")));
