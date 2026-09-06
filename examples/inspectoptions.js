// util.inspect's options, and the Map/Set layout they act on.
//
// A collection is laid out by the SAME routine as a plain object — it consults
// breakLength and compact like everything else — and the options an `assert`
// diff depends on (sorted, customInspect, showHidden, maxArrayLength) each
// change what is printed rather than only how it is spaced.
const util = require("util");

console.log(new Set(["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc", "dddddddddd", "eeeeeeeeee", "ffffffffff", "gggggggggg"]));
console.log(new Map([["alphaalpha", 1], ["bravobravo", 2], ["charliecharlie", 3], ["deltadelta", 4], ["echoecho", 5], ["foxtrotfox", 6]]));
console.log(new Set([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));

console.log(util.inspect(new Set([1, 2]), { compact: false }));
console.log(util.inspect(new Map([["a", 1]]), { compact: false }));
console.log(util.inspect([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], { compact: false }));
console.log(util.inspect([95, 34, 51, 66, 83, 49, 20, 21, 70, 85], { compact: 1 }));
console.log(util.inspect([new Date(0), null], { compact: 1 }));

console.log(util.inspect({ zz: 1, a: 2, m: 3 }, { sorted: true }));
console.log(util.inspect(Array.from({ length: 104 }, (_, i) => i), { maxArrayLength: Infinity }).split("\n").length);

const buf = Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]);
console.log(util.inspect(buf));
console.log(util.inspect(buf, { customInspect: false }));

console.log(util.inspect([1, 2], { showHidden: true }));
console.log(util.inspect([], { showHidden: true }));
console.log(util.inspect(new Uint8Array([1, 2]), { showHidden: true }));
console.log(util.inspect(/ab+c/gi, { showHidden: true }));
