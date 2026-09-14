// What the GLOBAL OBJECT holds, and what only looks like it does.
//
// Two ways the two were confused: a `globalThis.x` read fell back to the
// bare-identifier lookup, which walks the SCOPE CHAIN — so while any function
// was running its locals were readable off `globalThis`. And the ENTRY script's
// top-level `var`s were bound into the globals map, where node puts them in the
// CommonJS wrapper's scope; a REQUIRED module already behaved correctly, so
// only the entry file differed.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// A function's locals are not global-object properties, during the call or after.
function locals() { var v = 1; let l = 2; const c = 3; return [typeof globalThis.v, typeof globalThis.l, typeof globalThis.c, "v" in globalThis]; }
show("locals       ", () => locals());
show("distinctive  ", () => (function () { let zzq = 2; return [typeof globalThis.zzq, globalThis.zzq === undefined]; })());
// A top-level `var` or function declaration is a MODULE local here.
var topVar = 3;
function topFn() { return topVar; }
show("top-level    ", () => [typeof globalThis.topVar, typeof globalThis.topFn, "topVar" in globalThis]);
// …and still works as a binding: readable, writable, visible to closures.
show("still-bound  ", () => { topVar = 4; return [topVar, topFn()]; });
show("hoisting     ", () => [typeof laterVar, typeof laterFn]);
var laterVar = 5;
function laterFn() {}
// An IMPLICIT global — an assignment with no declaration — really is one.
show("implicit     ", () => { implicitOne = 9; return [typeof globalThis.implicitOne, implicitOne, "implicitOne" in globalThis]; });
// An explicit `globalThis.x = …` is one too, and the bare name reads it back.
show("explicit     ", () => { globalThis.explicitOne = 10; return [explicitOne, typeof globalThis.explicitOne]; });
// An implicit global is a real OWN property: `in`, `hasOwnProperty`, the key
// listings and `delete` all have to agree with the read, and none of them did.
show("own-property ", () => { implicitTwo = 11; return ["implicitTwo" in globalThis, globalThis.hasOwnProperty("implicitTwo"), Object.keys(globalThis).includes("implicitTwo"), JSON.stringify(Object.getOwnPropertyDescriptor(globalThis, "implicitTwo"))]; });
show("delete       ", () => { implicitThree = 12; const gone = delete globalThis.implicitThree; return [gone, typeof globalThis.implicitThree, "implicitThree" in globalThis]; });
// A BUILTIN is an own property too, but a non-enumerable one.
show("builtin-own  ", () => ["Math" in globalThis, globalThis.hasOwnProperty("Math"), Object.keys(globalThis).includes("Math"), Object.getOwnPropertyDescriptor(globalThis, "Math").enumerable]);
show("absent       ", () => ["nosuchglobal" in globalThis, globalThis.hasOwnProperty("nosuchglobal"), Object.getOwnPropertyDescriptor(globalThis, "nosuchglobal")]);
// The builtins a bare identifier resolves to are all real properties.
show("builtins     ", () => [typeof globalThis.Math, globalThis.Math === Math, typeof globalThis.process, globalThis.JSON === JSON]);
// The CommonJS wrapper's own parameters are locals, not properties.
show("cjs-locals   ", () => [typeof require, typeof globalThis.require, typeof module, typeof globalThis.module]);
// A DIRECT eval's `var` joins the enclosing function; an INDIRECT one is
// global code and binds on the global object.
show("direct-eval  ", () => { function o() { eval("var de = 1"); return [typeof de, typeof globalThis.de]; } return o(); });
show("indirect-eval", () => { const g = eval; g("var ie = 11"); return [typeof globalThis.ie, globalThis.ie]; });
show("eval-top     ", () => { eval("var et = 12"); return [et, typeof globalThis.et]; });
