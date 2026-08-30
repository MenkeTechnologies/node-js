// Primitive wrapper objects — `new String("a")`, `Object(1)`, and the boxing a
// sloppy-mode function does to its `this`. None of the three constructors
// existed ("String is not a constructor"), and `Object(1)` returned an empty
// object with no link to the primitive at all.

// The constructor form produces an OBJECT carrying the primitive.
const s = new String("ab"), n = new Number(3), b = new Boolean(false);
console.log("typeof  ", typeof s, typeof n, typeof b);
console.log("valueOf ", s.valueOf(), n.valueOf(), b.valueOf());
// A `Boolean(false)` wrapper is an object, so it is TRUTHY while comparing
// equal to `false` — the classic reason never to use one.
console.log("truthy  ", !!b, b == false, b === false, b ? "t" : "f");
// A String wrapper owns its index properties and a `length`, all non-writable.
console.log("indices ", s.length, s[0], s[1], s[2], Object.keys(s).join(","));
console.log("attrs   ", Object.getOwnPropertyDescriptor(s, "0").writable,
  Object.getOwnPropertyDescriptor(s, "length").enumerable,
  Object.getOwnPropertyNames(s).join(","));

// The prototype link is real, not a namespace handle: `instanceof` and
// `getPrototypeOf` both answer from the chain, for the wrapper AND the bare
// primitive.
console.log("proto   ", Object.getPrototypeOf(s) === String.prototype, s instanceof String,
  Object.getPrototypeOf(1) === Number.prototype, Object.getPrototypeOf(true) === Boolean.prototype);
console.log("typeofP ", typeof String.prototype, typeof Number.prototype);
console.log("ctor    ", s.constructor === String, "a".constructor === String);

// Methods resolve through the boxed primitive's own table.
console.log("methods ", new String("abc").charAt(1), new String("ab").toUpperCase(),
  new Number(1.5).toFixed(2), new Number(255).toString(16));
// The reflective Object.prototype methods still answer for the WRAPPER.
console.log("reflect ", s.hasOwnProperty("0"), s.hasOwnProperty("2"), Object.hasOwn(s, "length"));

// ToPrimitive unwraps, so arithmetic, concatenation and comparison all work.
console.log("coerce  ", n + 1, `${n}`, String(s), s + "c", new Number(5) > 3, new String("b") < "c");
// 20.1.3.6 brands by the internal slot, not by the object's shape.
console.log("brand   ", Object.prototype.toString.call(s), Object.prototype.toString.call(n),
  Object.prototype.toString.call(b));
// A String wrapper iterates its characters.
console.log("iterate ", [...new String("ab")].join("|"), Array.from(new String("ab")).join("|"));
// 25.5.2.2 step 4: a wrapper serializes as the primitive it boxes.
console.log("json    ", JSON.stringify([s, n, b]), JSON.stringify({ v: new String("x") }));
// util.inspect distinguishes a wrapper from its primitive, and still shows any
// extra own property — but not the boxed characters.
const tagged = new String("ab"); tagged.tag = 1;
console.log("inspect ", s, n, b, tagged, [new Number(2)]);

// `Object(v)` is ToObject: a primitive comes back boxed, an object unchanged,
// and a nullish argument becomes a fresh empty object.
const same = { k: 1 };
console.log("toObject", typeof Object(1), Object(1).valueOf(), Object("x").length,
  Object(same) === same, JSON.stringify(Object(null)), typeof Object(Symbol("q")));
console.log("newObj  ", typeof new Object("ab"), new Object("ab").valueOf(), new Object("ab") instanceof String);

// 10.2.1.2 OrdinaryCallBindThis: a SLOPPY function boxes a primitive `this`;
// a strict one receives it unchanged.
function sloppy() { return [typeof this, this instanceof Number, this.valueOf()].join(","); }
function strictly() { "use strict"; return [typeof this, this].join(","); }
console.log("boxthis ", sloppy.call(5), strictly.call(5));
// The same boxing is what makes a primitive `thisArg` usable in a callback.
console.log("thisArg ", [1].map(function () { return typeof this; }, "x").join(","));

// `class S extends String {}` links to the real String.prototype, so the
// subclass instance is branded and converts like a string.
class S extends String {}
const sub = new S("hi");
console.log("extend  ", sub.length, sub instanceof String, sub instanceof S, String(sub),
  Object.prototype.toString.call(sub));
