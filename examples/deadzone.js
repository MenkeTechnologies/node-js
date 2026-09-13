// A `let`/`const`/`class` binding exists from the top of its scope but cannot
// be read until its declaration runs — the temporal dead zone (8.1.1.1.6).
// There was no dead zone at all: the binding simply did not exist yet, so a
// read above the declaration either reported the name as UNDEFINED — the
// message for a typo — or, worse, silently found an OUTER binding of the same
// name and returned its value.
const t = (f) => { try { return "value " + JSON.stringify(f()); } catch (e) { return e.constructor.name + ": " + e.message; } };
let x = 1;
console.log("shadowed  ", t(() => { { const r = x; let x = 2; return r; } }));
console.log("const     ", t(() => { { c; const c = 1; } }));
console.log("class     ", t(() => { { new K(); class K {} } }));
// `typeof` does NOT excuse it: it answers "undefined" only for an UNBOUND name.
console.log("typeof    ", t(() => { { const r = typeof y; let y = 2; return r; } }));
console.log("typeof-un ", t(() => typeof neverDeclaredAnywhere));
// Assigning into the dead zone throws too — it is not an initialization.
console.log("assign    ", t(() => { { w = 1; let w; } }));
// The whole binding pattern is hoisted, not just a plain identifier.
console.log("destr-obj ", t(() => { { dz; let { dz } = { dz: 1 }; } }));
console.log("destr-arr ", t(() => { { da; let [da] = [1]; } }));
// A `for` head is its own scope, and so is a `switch` — one shared across all
// its cases, so a read from an earlier case is a dead-zone error.
console.log("for-head  ", t(() => { for (let i = i; false;) {} }));
console.log("switch    ", t(() => { switch (1) { case 1: s; let s; } }));

// What the dead zone must NOT disturb.
console.log("var       ", t(() => { { const r = typeof v; var v = 1; return r; } }));
function readsLater() { return LATER; }
const LATER = 7;
console.log("late-const", readsLater());
console.log("closure   ", t(() => { const g = () => cv; let cv = 3; return g(); }));
console.log("loop-body ", t(() => { const out = []; for (let i = 0; i < 2; i++) { let z = i * 2; out.push(z); } return out; }));
console.log("siblings  ", t(() => { const o = []; { let s = 1; o.push(s); } { let s = 2; o.push(s); } return o; }));
console.log("class-ok  ", t(() => { { class K2 { v() { return 1; } } return new K2().v(); } }));
console.log("switch-ok ", t(() => { switch (1) { case 1: { let sv = 5; return sv; } } }));
// A class's own name is bound inside its body while the static initializers
// run, and that inner binding SHADOWS the outer one still in its dead zone.
class C { static x = C.m(); static m() { return 5; } }
const K3 = class Inner { static self = Inner.name; };
console.log("class-self", C.x, K3.self);
// The marker backing all of this is not reachable from JavaScript: a top-level
// `const crypto` must not make `globalThis.crypto` read back as it.
const crypto = require("crypto");
console.log("no-leak   ", typeof globalThis.crypto, typeof crypto.createHash);
