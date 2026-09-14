// A direct `eval`'s two environments (19.2.1.1 steps 12-13). It shares the
// caller's VARIABLE environment — that is what lets `eval('var x=1')` inject a
// binding — but gets a LEXICAL one of its own. Both were the caller's, so a
// `let`, `const` or `class` declared inside leaked out, and one that shadowed
// an existing name overwrote it.
//
// Separately: a function declared in a BLOCK is block-scoped in strict mode.
// Only Annex B.3.3's sloppy legacy also hoists it to the function scope, and
// that was happening under `'use strict'` too.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// Lexical declarations die with the eval; `var` and function declarations do not.
show("lexical-out  ", () => { eval("let a1=1; const a2=2; class A3{}"); return [typeof a1, typeof a2, typeof A3]; });
show("var-out      ", () => { eval("var a4=1; function a5(){}"); return [typeof a4, typeof a5]; });
show("lexical-in   ", () => eval("let a6=6; a6"));
show("no-shadow    ", () => { let a7 = 1; eval("let a7=2"); return a7; });
show("in-function  ", () => { function o() { eval("let a8=1"); return typeof a8; } return o(); });
show("not-global   ", () => { eval("let a10=1"); return typeof globalThis.a10; });
// A STRICT direct eval keeps its `var`s too (19.2.1.1 step 12).
show("strict-var   ", () => { "use strict"; eval("var a9=1"); return typeof a9; });
// A sloppy direct eval still sees and writes the caller's bindings.
show("reads-caller ", () => { let v = 5; return eval("v + 1"); });
show("writes-caller", () => { let v = 5; eval("v = 6"); return v; });
show("this         ", () => { function o() { return eval("this") === globalThis; } return o(); });
// Annex B.3.3 in SLOPPY code: a block function is visible after the block.
show("sloppy-block ", () => { function o() { { function g() { return 5; } } return [typeof g, g()]; } return o(); });
show("sloppy-cond  ", () => { function o() { if (false) { function g() {} } return typeof g; } return o(); });
// …and is NOT in strict code, though it works inside its own block.
show("strict-out   ", () => { "use strict"; function o() { { function g() {} } return typeof g; } return o(); });
show("strict-in    ", () => { "use strict"; function o() { { function g() { return 1; } return g(); } } return o(); });
show("strict-hoist ", () => { "use strict"; function o() { { const b = typeof g; function g() {} return b; } } return o(); });
show("strict-two   ", () => { "use strict"; function o() { { function g() { return 1; } } { function g() { return 2; } } return typeof g; } return o(); });
show("strict-top   ", () => { "use strict"; function o() { function g() { return 3; } return g(); } return o(); });
show("strict-if    ", () => { "use strict"; function o() { if (true) { function g() { return 4; } return g(); } } return o(); });
show("strict-loop  ", () => { "use strict"; function o() { const r = []; for (let i = 0; i < 2; i++) { function g() { return i; } r.push(g()); } return r; } return o(); });
