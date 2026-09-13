// `Symbol.species` (and the well-known symbols generally). Nine of the fifteen
// well-known symbols did not exist, `Symbol.species` among them, so the species
// protocol had nothing to read: every derived result came back a plain builtin.
const names = ["iterator", "asyncIterator", "toPrimitive", "toStringTag", "hasInstance",
  "species", "isConcatSpreadable", "match", "matchAll", "replace", "search", "split",
  "unscopables", "dispose", "asyncDispose"];
console.log("symbols ", names.every((n) => typeof Symbol[n] === "symbol"), names.length);
// The accessor returns `this`, so each builtin is its own species.
console.log("builtins", Array[Symbol.species] === Array, Map[Symbol.species] === Map,
  Promise[Symbol.species] === Promise, RegExp[Symbol.species] === RegExp);

// A subclass that does not override it IS its own species. It used to read the
// accessor off the builtin ancestor and answer `Array`.
class MyArray extends Array {}
console.log("subclass", MyArray[Symbol.species] === MyArray, MyArray.name, MyArray.length);
// A class's own `name` and `length` are its own, not the ancestor's.
class Sized extends Array { constructor(a, b) { super(); } }
console.log("own     ", Sized.name, Sized.length, (class Plain { constructor(a) {} }).length);

// `Array.from`/`of` build through `this` (23.1.2.1), which is what starts the
// chain: without it the result was a plain array and nothing downstream could
// be a subclass either.
const from = MyArray.from([1, 2, 3]);
console.log("from    ", from instanceof MyArray, MyArray.of(1, 2) instanceof MyArray, from.join(","));
// ArraySpeciesCreate (23.1.3.4): every method that produces an array produces
// one of the receiver's species.
console.log("methods ", from.map((x) => x) instanceof MyArray, from.filter((x) => x > 1) instanceof MyArray,
  from.slice(1) instanceof MyArray, from.concat([4]) instanceof MyArray);
console.log("more    ", from.flat() instanceof MyArray, from.flatMap((x) => [x]) instanceof MyArray,
  MyArray.from([1, 2, 3]).splice(1, 1) instanceof MyArray);
// The constructor really runs — it is called with the LENGTH, then the elements
// are written, so a subclass constructor observes each allocation.
let allocations = 0;
class Counted extends Array { constructor(...a) { super(...a); allocations++; } }
const counted = Counted.from([1, 2]);
const mapped = counted.map((x) => x * 2);
console.log("ctor    ", mapped instanceof Counted, allocations, mapped.join(","));
// Overriding the accessor with `Array` opts out, which is the documented escape.
class Opted extends Array { static get [Symbol.species]() { return Array; } }
const opted = Opted.from([1, 2]);
console.log("opt-out ", opted.map((x) => x) instanceof Opted, opted.map((x) => x) instanceof Array);

// Every `Promise` combinator builds its result with `this` too (27.2.4.x).
class MyPromise extends Promise {}
console.log("promise ", MyPromise.resolve(1) instanceof MyPromise,
  MyPromise.all([1]) instanceof MyPromise, MyPromise.race([1]) instanceof MyPromise,
  MyPromise.allSettled([1]) instanceof MyPromise, MyPromise.any([1]) instanceof MyPromise);
// A rejected one still needs a handler, or the process reports it.
MyPromise.reject(new Error("x")).catch(() => {});
console.log("reject  ", MyPromise.reject(1).catch(() => {}) instanceof MyPromise);
// `then` uses the RECEIVER's species, so a chain stays in the subclass.
console.log("then    ", MyPromise.resolve(1).then((x) => x) instanceof MyPromise,
  MyPromise.resolve(1).then((x) => x).then((x) => x) instanceof MyPromise);
// The subclass constructor runs for these too.
let built = 0;
class Tracked extends Promise { constructor(f) { super(f); built++; this.tag = 1; } }
const tracked = Tracked.resolve(7);
console.log("track   ", tracked instanceof Tracked, tracked.tag, built > 0);
tracked.then((v) => console.log("value   ", v));

// A plain array and a plain promise are unaffected: their constructor is the
// builtin, whose species is itself.
console.log("plain   ", [1, 2].map((x) => x) instanceof Array,
  Object.getPrototypeOf([1, 2].map((x) => x)) === Array.prototype,
  Promise.resolve(1) instanceof Promise);

// Through a PROXY. An array method on a proxy takes the array-LIKE path: the
// elements are read into a temporary plain array and the method runs on that,
// so the species was computed from the temporary and every proxied subclass
// produced a plain `Array`. It has to come from the original receiver.
class Wrapped extends Array {}
const target = Wrapped.from([1, 2, 3]);
const proxied = new Proxy(target, {});
console.log("px-basic", Array.isArray(proxied), proxied.length, [...proxied].join(","));
console.log("px-spec ", proxied.map((x) => x) instanceof Wrapped,
  proxied.filter((x) => x > 1) instanceof Wrapped, proxied.slice(1) instanceof Wrapped);
// A proxy over a plain array still yields plain arrays.
const plainProxy = new Proxy([1, 2], {});
console.log("px-plain", plainProxy.map((x) => x) instanceof Array,
  Object.getPrototypeOf(plainProxy.map((x) => x)) === Array.prototype,
  [0].concat(plainProxy).join(","));
// The `constructor` a proxy reports comes from its `get` trap, which is what
// the species read has to go through.
let asked = 0;
const traced = new Proxy(target, {
  get(t, k, r) { if (k === "constructor") { asked++; } return Reflect.get(t, k, r); },
});
console.log("px-trap ", traced.map((x) => x) instanceof Wrapped, asked > 0);
