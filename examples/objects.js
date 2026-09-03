// Objects, destructuring, spread, and JSON.
const user = { name: "Ada", age: 36, roles: ["admin", "dev"] };
const { name, ...meta } = user;
console.log(name, meta);
const clone = { ...user, active: true };
console.log(Object.keys(clone), Object.values(meta));
console.log(JSON.stringify(clone));
const parsed = JSON.parse('{"a":1,"b":[2,3]}');
console.log(parsed.a, parsed.b);

// `util.inspect`'s depth option. `{ depth: null }` and `{ depth: Infinity }`
// both mean unlimited, which was stored as the maximum unsigned value — and the
// walk compares its indent against TWICE the depth, so doubling that overflowed
// and panicked the process. An abort, from an ordinary debugging call.
const util = require("util");
const deep = { a: { b: { c: { d: { e: 1 } } } } };
console.log("depth-def ", util.inspect(deep));
console.log("depth-0   ", util.inspect(deep, { depth: 0 }));
console.log("depth-1   ", util.inspect(deep, { depth: 1 }));
console.log("depth-null", util.inspect(deep, { depth: null }).replace(/\s+/g, " "));
console.log("depth-inf ", util.inspect(deep, { depth: Infinity }).replace(/\s+/g, " "));
// A NEGATIVE depth is legal and means "already past the limit". Only `null` and
// a non-finite depth mean unlimited; treating everything below zero as
// unlimited made this print the whole object instead of collapsing it.
console.log("depth-neg ", util.inspect(deep, { depth: -1 }), util.inspect(deep, { depth: -5 }));
console.log("depth-ninf", util.inspect(deep, { depth: -Infinity }), util.inspect([1, [2]], { depth: -1 }));
// Truncated toward zero, and the override lasts for the one call only.
console.log("depth-frac", util.inspect(deep, { depth: 1.9 }), util.inspect(deep));

// A Map/Set/Promise/RegExp/generator is an ordinary object that ALSO has
// internal slots, so it can carry own properties like anything else. Its heap
// representation held only those slots, so every such write vanished: `m.x = 5`
// left `m.x` undefined, and `Object.keys`, spread, `Object.assign`,
// `JSON.stringify` and `util.inspect` all reported nothing.
const m = new Map([["k", 1]]);
m.x = 5;
console.log("map     ", m.x, Object.keys(m).join(","), m.size, JSON.stringify({ ...m }));
const st = new Set([1]);
st.tag = "t";
console.log("set     ", st.tag, Object.getOwnPropertyNames(st).join(","), st.size, st.has(1));
const pr = Promise.resolve(1);
pr.p = 2;
const re = /a/g;
re.r = 3;
console.log("others  ", pr.p, re.r, Object.keys(re).join(","), re.source, re.flags);
// A RegExp's own `lastIndex` is still its match cursor, not a stored property.
re.lastIndex = 4;
console.log("cursor  ", re.lastIndex, Object.keys(re).join(","));
// defineProperty, hasOwn and JSON all see the same own properties.
Object.defineProperty(m, "d", { value: 3, enumerable: true });
console.log("define  ", m.d, Object.hasOwn(m, "d"), Object.hasOwn(m, "nope"),
  JSON.stringify(Object.assign({}, m)));
console.log("json    ", JSON.stringify(m), JSON.stringify(st));
// inspect shows the entries AND the attached properties.
console.log("inspect ", m, st, new Map(), new Set());
delete m.x;
console.log("delete  ", m.x, Object.keys(m).join(","));

// An EventEmitter keys a symbol event by the symbol itself, so `eventNames()`
// hands back something `off` accepts. It used to render the symbol as
// `"Symbol(desc)"`, collapsing it with a string event of that name and making
// every symbol listener unremovable by its symbol.
const EE = require("events");
const em = new EE();
const evt = Symbol("evt");
let seen = 0;
const fn = (n) => { seen += n; };
em.on(evt, fn);
em.on("plain", () => {});
const names = em.eventNames();
console.log("events  ", names.length, typeof names[0], typeof names[1], names[1] === evt);
em.emit(evt, 7);
console.log("emit    ", seen, em.listenerCount(evt), em.listeners(evt).length);
em.off(evt, fn);
em.emit(evt, 7);
console.log("off     ", seen, em.listenerCount(evt), em.eventNames().length);
// A string event of the symbol's DESCRIPTION is a different event entirely.
em.on("Symbol(evt)", () => { seen += 100; });
em.emit(evt, 1);
console.log("distinct", seen, em.eventNames().length);

// Every object INHERITS the `Object.prototype` methods, and an exotic that does
// not define its own reaches them the same way. Each kind's dispatch table only
// knew its own methods, so `new Map().toString()` reported "is not a function"
// while `Object.prototype.toString.call(m)` worked — the same method, reached
// two ways, disagreeing.
const exotics = [
  ["map ", new Map([[1, 2]])],
  ["set ", new Set([1])],
  ["prom", Promise.resolve(1)],
  ["re  ", /x/g],
  ["gen ", (function* () {})()],
  ["sym ", Symbol("s")],
  ["big ", 1n],
];
for (const [label, v] of exotics) {
  console.log("inherit " + label, v.toString(), v.hasOwnProperty("nope"),
    v.propertyIsEnumerable("x"), typeof v.toLocaleString());
}
// `toString` goes through the branded form, so it reads the receiver's own
// brand rather than stringifying it generically.
console.log("brand   ", new Map().toString(), new Set().toString(),
  Promise.resolve().toString(), (function* () {})().toString());
// A `Symbol.toStringTag` still wins over the brand.
class Tagged { get [Symbol.toStringTag]() { return "Custom"; } }
console.log("tag     ", Object.prototype.toString.call(new Tagged()), String(new Tagged()));
// `valueOf` returns the receiver for anything that does not override it.
const sym = Symbol("v");
console.log("valueOf ", sym.valueOf() === sym, /x/.valueOf() instanceof RegExp,
  new Map().valueOf() instanceof Map);
// The kinds that DO define their own keep them: an Array stringifies its
// elements, a RegExp its source, a Number in the requested radix.
console.log("own     ", [1, 2, 3].toString(), /a+/g.toString(), (255).toString(16),
  (1.5).toString(), true.toString(), "s".toString());
console.log("locale  ", [1, 2].toLocaleString(), (1234.5).toString());
