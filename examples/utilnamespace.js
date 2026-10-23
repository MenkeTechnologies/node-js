// The shape of `require('util')`. `Object.keys(util)` listed seven dotted
// dispatch paths (`types.isMap` … `types.isAsyncFunction`) as if they were
// keys of `util`, and `util.types` was a FUNCTION (`typeof` said so, and it
// branded `[object Function]`) where node's is a plain object.
const util = require("util");
console.log("dotted ", Object.keys(util).filter((k) => k.includes(".")).length);
console.log("types  ", Object.keys(util).includes("types"), typeof util.types, Object.prototype.toString.call(util.types));
console.log("same   ", util.types === require("util/types"), util.types instanceof Function);
console.log("member ", typeof util.types.isMap, util.types.isMap(new Map()), util.types.isDate(new Date()));
console.log("procdot", Object.keys(process).filter((k) => k.includes(".")).length);
