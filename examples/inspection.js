// `util.inspect`'s LAYOUT rules. Whether a group prints on one line is not just
// a length question: node's `reduceToSingleString` also refuses to join a group
// whose subtree is `compact` levels deep or more (default 3). Only the length
// half was implemented, so a deeply nested object printed on one line where
// node breaks the outer levels, and the `compact` and `breakLength` options
// were ignored outright.
const u = require("util");
const p = (label, x) => console.log(label, JSON.stringify(x));

// The depth gate. Node tracks the level of the value it most recently EXPANDED
// and compares it against each group's own level as the render unwinds.
p("d1      ", u.inspect({ a: 1 }));
p("d3      ", u.inspect({ a: { b: { c: 1 } } }));
// Four levels deep is exactly the threshold, so the OUTERMOST group breaks and
// everything below it still fits on one line.
p("d4-null ", u.inspect({ a: { b: { c: { d: 1 } } } }, { depth: null }));
p("d5-null ", u.inspect({ a: { b: { c: { d: { e: 1 } } } } }, { depth: null }));
// At the default depth of 2 the fourth level is elided to `[Object]`, and an
// ELIDED level does not count toward the gate — so the same object joins.
p("d4-dflt ", u.inspect({ a: { b: { c: { d: 1 } } } }));
p("d4-at3  ", u.inspect({ a: { b: { c: { d: 1 } } } }, { depth: 3 }));
p("d4-at1  ", u.inspect({ a: { b: { c: { d: 1 } } } }, { depth: 1 }));
p("arrays  ", u.inspect([[[[1]]]], { depth: null }));
p("mixed   ", u.inspect({ a: [1, 2, { b: { c: 1 } }] }, { depth: null }));

// `compact` moves the threshold, and `false` disables joining entirely.
p("c1      ", u.inspect({ a: { b: { c: { d: 1 } } } }, { depth: null, compact: 1 }));
p("c5      ", u.inspect({ a: { b: { c: { d: 1 } } } }, { depth: null, compact: 5 }));
p("c1-flat ", u.inspect({ a: { b: 1 } }, { compact: 1 }));
p("false   ", u.inspect([1, 2], { compact: false }));
p("false-o ", u.inspect({ x: 1, y: 2 }, { compact: false }));
p("false-n ", u.inspect({ a: { b: 1 }, c: [1, 2] }, { compact: false }));
// An empty group has nothing to break.
p("false-e ", u.inspect([], { compact: false }) + u.inspect({}, { compact: false }));

// `breakLength` is the column budget for the joined line.
p("bl-10   ", u.inspect({ aaa: 1, bbb: 2, ccc: 3 }, { breakLength: 10 }));
p("bl-200  ", u.inspect({ aaa: 1, bbb: 2, ccc: 3 }, { breakLength: 200 }));
p("bl-1    ", u.inspect({ a: 1, b: 2 }, { breakLength: 1 }));
p("bl-grid ", u.inspect(Array.from({ length: 8 }, (_, i) => i), { breakLength: 20 }));

// Combined, and console.log's own defaults are unaffected by any of it.
p("combo   ", u.inspect([1, 2], { compact: false, depth: null }));
console.log("console ", { a: { b: { c: { d: 1 } } } }, [1, 2, 3], { x: { y: { z: { w: 1 } } } });
