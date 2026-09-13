// The `in` operator could not see an INHERITED builtin method. These are not
// objects on the prototype chain — the read path synthesizes them from the
// intrinsic table — so nothing `in` consulted knew about them, and it disagreed
// with what a read gives for every builtin method of every builtin kind.
// `'then' in x` is the standard thenable test, and it answered false for a
// promise.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
show("object   ", () => ["toString" in {}, "valueOf" in {}, "hasOwnProperty" in {}, "constructor" in {}, "nope" in {}]);
show("array    ", () => ["push" in [], "map" in [], "length" in [], "nope" in []]);
show("function ", () => ["call" in function () {}, "bind" in function () {}, "name" in function () {}]);
show("promise  ", () => ["then" in Promise.resolve(), "catch" in Promise.resolve()]);
show("map      ", () => ["get" in new Map(), "size" in new Map()]);
show("date     ", () => ["getTime" in new Date(), "toISOString" in new Date()]);
show("regexp   ", () => ["test" in /a/, "lastIndex" in /a/, "source" in /a/]);
show("error    ", () => ["message" in new Error("m"), "toString" in new Error(), "stack" in new Error()]);
show("wrapper  ", () => ["slice" in new String("a"), "length" in new String("a"),
  "description" in Object(Symbol("x")), "toString" in Object(Symbol("x"))]);
// An ACCESSOR member counts — `size`, `source` and `description` are members but
// not functions, which is why the function table alone cannot answer.
show("accessors", () => ["size" in new Set(), "flags" in /a/, "byteLength" in new ArrayBuffer(1)]);
// A user class inherits both its own and Object.prototype's.
show("subclass ", () => { class A { m() {} } class B extends A {} return ["m" in new B(), "toString" in new B()]; });
show("chain    ", () => { const p = { a: 1 }; return ["a" in Object.create(p), "toString" in Object.create(p)]; });
// A null-prototype object inherits NOTHING.
show("nullproto", () => { const o = Object.create(null); return ["toString" in o, "x" in Object.assign(o, { x: 1 })]; });

// `delete` of a NON-CONFIGURABLE property answers false. A member of a builtin
// namespace has no property map behind it, so the attribute lookup could not
// tell and every one of them reported success.
show("delete-ns", () => [delete Math.PI, delete Object.prototype, delete Number.MAX_VALUE, Math.PI === 3.141592653589793]);
show("delete-own", () => { const o = {}; Object.defineProperty(o, "f", { value: 1 }); return [delete o.f, "f" in o]; });
show("delete-ok", () => { const o = { a: 1 }; return [delete o.a, delete o.missing, "a" in o]; });

// In STRICT mode a refused delete throws instead of answering false.
show("strict   ", () => {
  "use strict";
  const out = [];
  const o = {};
  Object.defineProperty(o, "f", { value: 1 });
  for (const f of [() => delete o.f, () => delete o["f"], () => delete Math.PI,
    () => delete [1].length, () => delete Object.freeze({ a: 1 }).a]) {
    try { f(); out.push("no throw"); } catch (e) { out.push(e.message); }
  }
  // A missing property and a configurable one are both fine.
  out.push(delete o.nope, delete { a: 1 }.a);
  return out;
});

// A STRICT direct `eval` gets its own variable environment, so its `var`s and
// function declarations die with it — only a SLOPPY one shares the caller's.
// Sharing it unconditionally left the binding behind.
show("strict-eval", () => { "use strict"; eval("var sv = 1"); return typeof sv; });
show("strict-fn  ", () => { "use strict"; eval("function sf() {}"); return typeof sf; });
show("strict-let ", () => { "use strict"; eval("let sl = 1"); return typeof sl; });
show("by-source  ", () => { eval("'use strict'; var ss = 1"); return typeof ss; });
show("nested     ", () => { "use strict"; return eval("eval('var nn = 1'); typeof nn"); });
show("sloppy     ", () => { eval("var lv = 1"); return typeof lv; });
show("sloppy-fn  ", () => { eval("function lf() {}"); return typeof lf; });
// It still READS and WRITES the enclosing scope, and still yields its value.
show("reads      ", () => { "use strict"; const ro = 7; return eval("ro"); });
show("writes     ", () => { "use strict"; let wo = 1; eval("wo = 2"); return wo; });
show("value      ", () => { "use strict"; return eval("var q = 3; q"); });
