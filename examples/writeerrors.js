// The WRITE side of the most common runtime fault in JS. `u.x = 1` on a nullish
// receiver was silently discarded — the read side already reported `Cannot read
// properties of undefined (reading 'x')`, so the two halves of the same
// mistake behaved completely differently.
//
// A PRIMITIVE receiver is the other half: `ToObject` makes a throwaway wrapper,
// so the write goes nowhere. Sloppy code discards it (as node does), strict
// code throws — and that refusal was silent too.
const show = (label, src) => {
  try { eval(src); console.log(label, "no throw"); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const u = undefined, nl = null, o = {};

// Every shape of write names the key it was setting.
show("dot-undefined", "u.x = 1");
show("dot-null     ", "nl.x = 1");
show("index-number ", "u[0] = 1");
show("index-string ", "u['k'] = 1");
show("computed     ", "const k = 'z'; u[k] = 1");
show("deep         ", "o.a.b = 1");
// A compound assignment and an increment fail on the READ first.
show("compound     ", "u.x += 1");
show("increment    ", "u.x++");
// `delete` runs ToObject on the base, which a nullish one refuses.
show("delete-undef ", "delete u.x");
show("delete-null  ", "delete nl.x");
// A PRIMITIVE receiver: silent in sloppy code, named in strict — including the
// three that ride as heap handles here (string, symbol, bigint), which a
// shape-based test for a primitive misses.
show("sloppy-number", "(5).x = 1");
show("strict-number", "(function () { 'use strict'; (5).x = 1; })()");
show("strict-string", "(function () { 'use strict'; 'ab'.x = 1; })()");
show("strict-bool  ", "(function () { 'use strict'; true.x = 1; })()");
show("strict-symbol", "(function () { 'use strict'; Symbol('s').x = 1; })()");
show("strict-bigint", "(function () { 'use strict'; (1n).x = 1; })()");
// …and none of it disturbs a legitimate write.
show("plain        ", "const a = {}; a.x = 1; if (a.x !== 1) throw new Error('bad');");
show("array        ", "const a = []; a[0] = 1; a.length = 2; if (a.length !== 2) throw new Error('bad');");
show("boxed        ", "const s = Object('ab'); s.x = 1; if (s.x !== 1) throw new Error('bad');");
show("setter       ", "const a = { set v(x) { this.got = x; } }; a.v = 5; if (a.got !== 5) throw new Error('bad');");
show("inherited    ", "const a = Object.create({}); a.x = 1; if (a.x !== 1) throw new Error('bad');");
show("delete-plain ", "const a = { x: 1 }; if (!(delete a.x)) throw new Error('bad');");
show("delete-prim  ", "delete (5).x");
show("optional     ", "u?.x");
