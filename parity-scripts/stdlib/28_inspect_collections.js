// util.inspect: Map/Set laid out by the same rules as an object, and the
// options an `assert` diff depends on (compact / sorted / maxArrayLength /
// customInspect / showHidden).
const util = require("util");

// A collection wide enough to pass breakLength breaks, one member per line —
// and a Set is never column-grouped the way an array is.
console.log(new Set(["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc", "dddddddddd", "eeeeeeeeee", "ffffffffff", "gggggggggg"]));
console.log(new Map([["alphaalpha", 1], ["bravobravo", 2], ["charliecharlie", 3], ["deltadelta", 4], ["echoecho", 5], ["foxtrotfox", 6]]));
console.log(new Set([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));
console.log(new Set([1, 2]), new Map([["a", 1]]), new Set(), new Map());

// compact:false expands every group, including collections, and turns the
// array column grid off entirely.
console.log(util.inspect(new Set([1, 2]), { compact: false }));
console.log(util.inspect(new Map([["a", 1]]), { compact: false }));
console.log(util.inspect([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], { compact: false }));
// compact:N also caps the grid width at N*4 columns.
console.log(util.inspect([95, 34, 51, 66, 83, 49, 20, 21, 70, 85], { compact: 1 }));
// A leaf-rendered member (a bare Date) costs the group no nesting level.
console.log(util.inspect([new Date(0), null], { compact: 1 }));

// sorted orders the rendered entries; maxArrayLength lifts the 100-element cap.
console.log(util.inspect({ zz: 1, a: 2, m: 3 }, { sorted: true }));
console.log(util.inspect(Array.from({ length: 104 }, (_, i) => i), { maxArrayLength: Infinity }).split("\n").length);
console.log(util.inspect(Array.from({ length: 104 }, (_, i) => i)).endsWith("more items\n]"));

// customInspect:false drops Buffer's own rendering for the generic byte view.
const buf = Buffer.from([0xff, 0xfe, 0x00, 0x41, 0x80]);
console.log(util.inspect(buf));
console.log(util.inspect(buf, { customInspect: false }));
console.log(util.inspect(buf, { customInspect: false, depth: -1 }));

// showHidden reveals the non-enumerable slots.
console.log(util.inspect([1, 2], { showHidden: true }));
console.log(util.inspect([], { showHidden: true }));
console.log(util.inspect(Object.assign([1], { x: 2 }), { showHidden: true }));
console.log(util.inspect(new Uint8Array([1, 2]), { showHidden: true }));
console.log(util.inspect(/ab+c/gi, { showHidden: true }));
