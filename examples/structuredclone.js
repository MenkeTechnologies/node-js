// `structuredClone` was a deep copy that shared anything it did not recognise
// and read values straight out of the property map. Three consequences, each
// silent: a non-cloneable value came back BY REFERENCE instead of throwing, an
// accessor property vanished, and every object kept its prototype.

// A value the algorithm refuses throws a DataCloneError naming it. These used
// to be handed back as the same object.
const refuse = (n, v) => {
  try { const r = structuredClone(v); console.log("ok    ", n, r === v ? "SAME OBJECT" : "cloned"); }
  catch (e) { console.log("throw ", n, e.constructor.name, e.name, e.code, e.message); }
};
refuse("symbol", Symbol("s"));
refuse("symbol-nodesc", Symbol());
refuse("weakmap", new WeakMap());
refuse("weakset", new WeakSet());
refuse("weakref", new WeakRef({}));
refuse("promise", Promise.resolve());
refuse("generator", (function* () {})());
refuse("proxy", new Proxy({ a: 1 }, {}));
// A function is refused too, and so is one nested anywhere in the graph. The
// MESSAGE quotes the function's source, which this frontend does not retain, so
// only the error itself is pinned here.
for (const [n, v] of [["function", () => {}], ["nested", { a: 1, deep: { f() {} } }]]) {
  try { structuredClone(v); console.log("ok    ", n); }
  catch (e) { console.log("throw ", n, e.constructor.name, e.name, e.message.endsWith("could not be cloned.")); }
}

// An ACCESSOR is read through — it used to be dropped entirely, because the
// property map holds nothing for one — and arrives as a plain data property.
let reads = 0;
const fromGetter = structuredClone({ get p() { reads++; return 7; } });
console.log("getter", fromGetter.p, reads,
  JSON.stringify(Object.getOwnPropertyDescriptor(fromGetter, "p")));
// A symbol key and a non-enumerable property are dropped.
console.log("dropped",
  Object.getOwnPropertySymbols(structuredClone({ [Symbol("k")]: 1 })).length,
  JSON.stringify(Object.keys(structuredClone(Object.defineProperty({}, "h", { value: 1 })))));

// The prototype survives only for the exotics the algorithm reproduces.
const kind = (v) => { const c = structuredClone(v); return Object.prototype.toString.call(c) + " " + c.constructor.name; };
class Plain { constructor() { this.k = 1; } }
class SubArray extends Array {}
class SubError extends Error {}
console.log("user-class ", kind(new Plain()), structuredClone(new Plain()) instanceof Plain);
console.log("array-sub  ", kind(new SubArray()));
console.log("error-sub  ", kind(new SubError("m")));
console.log("error      ", kind(new TypeError("t")), structuredClone(new TypeError("t")) instanceof TypeError);
console.log("date       ", kind(new Date(0)), +structuredClone(new Date(0)));
console.log("regexp     ", kind(/a/gi));
console.log("boxed      ", kind(new Number(5)), kind(new Boolean(true)), kind(new String("s")));
console.log("views      ", kind(new Uint8Array([1])), kind(new DataView(new ArrayBuffer(2))), kind(new ArrayBuffer(2)));
// A Buffer is NOT reproduced as a Buffer — node hands back a plain Uint8Array.
const clonedBuffer = structuredClone(Buffer.from([1, 2]));
console.log("buffer     ", clonedBuffer.constructor.name, Buffer.isBuffer(clonedBuffer),
  Object.prototype.toString.call(clonedBuffer), clonedBuffer.length, clonedBuffer[0]);
// An error clones its name, message and stack and nothing else.
console.log("error-props", structuredClone(Object.assign(new TypeError("t"), { extra: 1 })).extra,
  structuredClone(new TypeError("t")).message);

// The clone is INDEPENDENT of the source, which sharing the object was not.
const re = /a/g; re.lastIndex = 5;
const reClone = structuredClone(re); reClone.lastIndex = 0;
console.log("regexp-ind ", re.lastIndex, reClone.lastIndex, reClone.source, reClone.flags);
const ta = new Uint8Array([1, 2]); const taClone = structuredClone(ta); taClone[0] = 9;
console.log("view-ind   ", ta[0], taClone[0]);
const ab = new ArrayBuffer(2); new Uint8Array(ab)[0] = 1;
const abClone = structuredClone(ab); new Uint8Array(abClone)[0] = 9;
console.log("buffer-ind ", new Uint8Array(ab)[0], new Uint8Array(abClone)[0], abClone.byteLength);

// The reference GRAPH is preserved, which already worked and must keep working.
const cyclic = {}; cyclic.self = cyclic;
const shared = { v: 1 };
const both = structuredClone({ a: shared, b: shared });
console.log("graph      ", structuredClone(cyclic).self === structuredClone(cyclic), both.a === both.b, both.a !== shared);
const deep = structuredClone({ n: 1, s: "a", b: true, nul: null, u: undefined, big: 1n,
  m: new Map([[1, { x: 2 }]]), st: new Set([1, 2]), arr: [1, [2]] });
console.log("values     ", deep.n, deep.s, deep.b, deep.nul, "u" in deep, typeof deep.big,
  deep.m.get(1).x, [...deep.st].join(""), deep.arr[1][0]);
