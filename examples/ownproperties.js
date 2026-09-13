// The five views of "does this property exist" have to agree: a READ, `in`,
// `hasOwnProperty`, enumeration, and the DESCRIPTOR. For the properties a
// function, a typed array and a RegExp SYNTHESIZE rather than store in a
// property map, they did not — a read answered a value while
// `Object.getOwnPropertyDescriptor(f, 'name')` said no such property, which is
// exactly what a shim checks before patching something.
const views = (label, o, k) => console.log(label.padEnd(18),
  typeof o[k], k in o, Object.prototype.hasOwnProperty.call(o, k),
  Object.keys(o).includes(k), Object.getOwnPropertyDescriptor(o, k) !== undefined,
  Object.getOwnPropertyNames(o).includes(k));
console.log("                   read  in    own   keys  desc  gopn");
views("fn.length", function (a) {}, "length");
views("fn.name", function nm() {}, "name");
views("fn.prototype", function () {}, "prototype");
views("arrow.prototype", () => {}, "prototype");
views("class.prototype", class {}, "prototype");
views("regexp.lastIndex", /a/g, "lastIndex");
views("u8[0]", new Uint8Array([1]), "0");
views("buffer[0]", Buffer.from([1]), "0");
views("array.length", [1, 2], "length");
views("array[0]", [1, 2], "0");
views("string.length", new String("ab"), "length");
views("error.message", new Error("m"), "message");
views("error.stack", new Error("m"), "stack");
views("arguments[0]", (function () { return arguments; })(1), "0");
views("map.size", new Map(), "size");
views("inherited", Object.create({ p: 1 }), "p");

// The exact attributes, which differ per kind: a callable's `length`/`name` are
// read-only but configurable, its `prototype` is writable and NOT configurable,
// and a CLASS's is neither. An arrow, a method and a bound function own no
// `prototype` at all.
const desc = (o, k) => { const d = Object.getOwnPropertyDescriptor(o, k); return d && JSON.stringify({ ...d, value: typeof d.value === "object" ? "<obj>" : d.value }); };
function two(a, b) {}
console.log("length   ", desc(two, "length"));
console.log("name     ", desc(two, "name"));
console.log("prototype", desc(two, "prototype"));
console.log("class    ", desc(class {}, "prototype"));
console.log("no-proto ", desc(() => {}, "prototype"), desc({ m() {} }.m, "prototype"),
  desc(two.bind(null), "prototype"));
console.log("lastIndex", desc(/a/g, "lastIndex"));
console.log("element  ", desc(new Uint8Array([7]), "0"), desc(new Uint8Array([7]), "1"));

// …and the NAME lists they imply.
const names = (o) => JSON.stringify(Object.getOwnPropertyNames(o));
console.log("names    ", names(two), names(() => {}), names(class {}),
  names(/a/g), names(new Uint8Array([1, 2])), names(two.bind(null)));

// `defineProperty` has to write THROUGH to the exotic, not store a shadowing
// map entry the read never consults.
const target = new Uint8Array(2);
Object.defineProperty(target, "0", { value: 9 });
const re = /a/g;
Object.defineProperty(re, "lastIndex", { value: 5 });
console.log("define   ", target[0], re.lastIndex, /b/g.lastIndex);
// Redefining a function's own metadata still works.
function named() {}
Object.defineProperty(named, "name", { value: "z" });
console.log("redefine ", named.name, desc(named, "name"));
// And the descriptor set round-trips.
console.log("descs    ", JSON.stringify(Object.keys(Object.getOwnPropertyDescriptors(two)).sort()),
  JSON.stringify(Object.keys(Object.getOwnPropertyDescriptors(new Uint8Array([1, 2])))));
// None of them are ENUMERABLE, so copying sees nothing.
console.log("copies   ", JSON.stringify(Object.assign({}, two)), JSON.stringify({ .../a/g }),
  JSON.stringify(Object.entries(new Uint8Array([5, 6]))), JSON.stringify(new Uint8Array([5, 6])));
