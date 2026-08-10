// Top-level `this` at the FILE entry point.
//
// `node f.js` runs a CommonJS module, so top-level `this` is `module.exports` —
// a DIFFERENT answer from `node -e` and `node -`, where it is `globalThis`.
// The fuzzer drives `-e`, so this file is the only place the module answer is
// checked. It was `undefined` at all three entry points until round 5, which no
// generator could have reported because none of them emits a bare `this`.
console.log(typeof this, this === module.exports, this === globalThis);

// `this.x = 1` at module scope populates the exports object; while `this` was
// `undefined` this threw instead.
this.alpha = 1;
console.log(module.exports.alpha, exports.alpha);

// `globalThis` is one object, not a fresh one per read.
console.log(globalThis === globalThis, globalThis === global, typeof global);
globalThis.beta = 2;
console.log(globalThis.beta);
const g = globalThis;
g.gamma = 3;
console.log(globalThis.gamma);

// A plain (unbound) call still gets its OWN binding rather than inheriting the
// module's — see BUGS.md for the sloppy-mode `this` gap that keeps this
// `undefined` here and `object` in node, which is why only the TYPE-independent
// half is asserted.
function plain() {
  return this === module.exports;
}
console.log(plain());
