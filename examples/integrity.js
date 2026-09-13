// `Object.freeze`/`seal` and the `Object.prototype` methods every receiver
// inherits — two more places where one view of an object disagreed with
// another.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// Every object reaches an `Object.prototype` method it does not override. Each
// kind's READ arm knew only its own, so `typeof new Map().toString` was
// `undefined` while `new Map().toString()` WORKED — the read and the dispatch
// disagreeing about the same method — and on a function, a class, a RegExp or
// a Promise neither one resolved.
const OBJECT_METHODS = ["hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable",
  "toLocaleString", "valueOf", "toString"];
for (const [name, recv] of [["function", function f() {}], ["arrow", () => {}],
  ["class", class {}], ["regexp", /a/g], ["uint8", new Uint8Array([1])],
  ["map", new Map()], ["date", new Date(0)], ["promise", Promise.resolve()],
  ["error", new Error("m")], ["symbol", Object(Symbol("s"))]]) {
  console.log(("reads-" + name).padEnd(16), JSON.stringify(OBJECT_METHODS.map((m) => typeof recv[m])));
}
// …and calling one works, with the receiver's OWN version still winning where
// it defines one (a Map's `toString` is the branded form, not the generic).
function fn() {}
console.log("calls           ", fn.hasOwnProperty("name"), fn.propertyIsEnumerable("name"),
  new Map().toString(), /a/g.hasOwnProperty("lastIndex"), Promise.resolve().hasOwnProperty("x"));

// `Object.freeze` has to reach the own properties each shape actually keeps —
// a RegExp's `lastIndex` lives in its struct and a function's statics in a side
// table, so freezing sealed NOTHING for either and both stayed writable.
show("frozen-regexp  ", () => { const r = /a/g; Object.freeze(r); r.lastIndex = 5; return r.lastIndex; });
show("frozen-static  ", () => { const f = function () {}; f.s = 1; Object.freeze(f); f.s = 2; return f.s; });
show("frozen-map-prop", () => { const m = Object.assign(new Map(), { a: 1 }); Object.freeze(m); m.a = 2; return m.a; });
// Freezing does NOT close a Map's entries — those are internal slots.
show("frozen-entries ", () => { const m = new Map([[1, 2]]); Object.freeze(m); m.set(3, 4); return m.size; });
// `preventExtensions` alone leaves existing properties writable.
show("prevented      ", () => { const r = Object.preventExtensions(/c/g); r.lastIndex = 4; return r.lastIndex; });
show("sealed-add     ", () => { const f = function () {}; Object.seal(f); f.n = 1; return f.n; });
show("sealed-modify  ", () => { const f = function () {}; f.s = 1; Object.seal(f); f.s = 2; return f.s; });
for (const [name, make] of [["object", () => ({ a: 1 })], ["array", () => [1, 2]],
  ["function", () => { const f = function () {}; f.s = 1; return f; }],
  ["regexp", () => /a/g], ["map", () => Object.assign(new Map([[1, 2]]), { a: 1 })],
  ["promise", () => Object.assign(Promise.resolve(), { a: 1 })],
  ["error", () => new Error("m")], ["empty-view", () => new Uint8Array(0)],
  ["dataview", () => new DataView(new ArrayBuffer(2))]]) {
  show(("levels-" + name).padEnd(16), () => { const o = make(); Object.freeze(o); return [Object.isFrozen(o), Object.isSealed(o), Object.isExtensible(o)]; });
}
// A typed array WITH elements cannot be frozen or sealed at all: its indices
// are non-configurable by construction, so node refuses rather than
// half-applying the operation. An EMPTY one and a DataView are fine.
show("view-freeze    ", () => { Object.freeze(new Uint8Array(1)); return "ok"; });
show("buffer-freeze  ", () => { Object.freeze(Buffer.from([1])); return "ok"; });
show("view-seal      ", () => { Object.seal(new Uint8Array(1)); return "ok"; });

// In STRICT code a refused write throws, and the receiver is named by its
// BRAND. Only Array was special-cased, so every other exotic said `#<Object>`.
// Which of the two messages applies turns on whether the key already exists.
function strictWrites() {
  "use strict";
  const out = [];
  const attempt = (o, k) => { try { Object.freeze(o); o[k] = 9; out.push("no throw"); } catch (e) { out.push(e.message); } };
  attempt({ a: 1 }, "a");
  attempt([1], "0");
  attempt(/a/g, "lastIndex");
  attempt(new Date(0), "x");
  attempt(function f() {}, "x");
  attempt(new Error("m"), "message");
  attempt(Object.assign(new Map(), { a: 1 }), "a");
  attempt(new (class C { constructor() { this.a = 1; } })(), "a");
  return out;
}
console.log(strictWrites().join("\n"));
