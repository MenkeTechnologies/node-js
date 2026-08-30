// `thisArg` on the iteration methods, and the sloppy-mode `this` substitution.
// Both were missing, and between them every callback in the language saw the
// wrong `this`.

// `[1].forEach(fn, thisArg)` binds thisArg as the callback's `this`, and so do
// map/filter/some/every/find/findIndex/findLast/findLastIndex/flatMap,
// Map/Set/TypedArray forEach, and Array.from's map function. The argument was
// accepted and dropped, so `this` inside the callback was undefined.
const host = { tag: "T" };
const readTag = function () { return this && this.tag; };
const saw = [];
[1].forEach(function () { saw.push(this.tag); }, host);
console.log("forEach ", saw.join(","));
console.log("array   ", [1].map(readTag, host)[0], [1].filter(function () { return this.tag === "T"; }, host).length);
console.log("search  ", [1].some(readTag, host), [1].every(readTag, host), [1].find(readTag, host), [1].findIndex(readTag, host));
console.log("last    ", [1].findLast(readTag, host), [1].findLastIndex(readTag, host), [1].flatMap(readTag, host)[0]);
console.log("from    ", Array.from([1], readTag, host)[0]);
const mapSaw = [];
new Map([[1, "a"]]).forEach(function () { mapSaw.push(this.tag); }, host);
new Set([1]).forEach(function () { mapSaw.push(this.tag); }, host);
new Uint8Array([1]).forEach(function () { mapSaw.push(this.tag); }, host);
console.log("colls   ", mapSaw.join(","));
console.log("typed   ", new Uint8Array([1]).map(function () { return this.tag === "T" ? 9 : 0; }, host)[0]);
// forEach's callback also receives (value, index, collection).
const shape = [];
new Map([[1, "a"]]).forEach((v, k, m) => shape.push(v, k, m.size));
new Set(["x"]).forEach((v, v2, s) => shape.push(v, v2, s.size));
console.log("args    ", shape.join(","));

// A SLOPPY function called with no receiver gets the GLOBAL object as `this`
// (10.2.1.2); only a strict one keeps undefined. It was undefined everywhere,
// so a detached method, a bare call and `fn.call(null)` all disagreed with node.
function whoAmI() { return this === undefined ? "undefined" : this === globalThis ? "global" : typeof this; }
console.log("plain   ", whoAmI(), whoAmI.call(undefined), whoAmI.call(null));
console.log("detached", (() => { const o = { m: whoAmI }; const loose = o.m; return loose(); })());
console.log("callback", [1].map(whoAmI)[0], whoAmI.bind(null)());
// An explicit object receiver is untouched, and a method call still gets its
// object.
console.log("receiver", whoAmI.call({ a: 1 }), ({ m: whoAmI }).m() === "global" ? "global" : "object");
// A strict function keeps undefined, and an arrow has no `this` to substitute.
console.log("strict  ", (() => { "use strict"; function s() { return this === undefined ? "undefined" : "other"; } return s(); })());
console.log("arrow   ", (() => { const a = () => (this === undefined ? "undefined" : typeof this); return a(); })());
