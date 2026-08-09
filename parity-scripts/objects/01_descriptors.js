// Object property descriptors: defineProperty flags, accessors, and the
// enumeration surface (keys/values/entries/for-in/JSON) that depends on them.
const o = {};
Object.defineProperty(o, "hidden", { value: 1, enumerable: false, writable: false, configurable: false });
Object.defineProperty(o, "shown", { value: 2, enumerable: true, writable: true, configurable: true });
Object.defineProperty(o, "computed", { get() { return this.shown * 10; }, enumerable: true, configurable: true });
o.plain = 3;

console.log(JSON.stringify(Object.keys(o)));
console.log(JSON.stringify(Object.getOwnPropertyNames(o).sort()));
console.log(JSON.stringify(Object.values(o)));
console.log(JSON.stringify(Object.entries(o)));
console.log(JSON.stringify(o));
console.log(JSON.stringify({ ...o }));

const inKeys = [];
for (const k in o) inKeys.push(k);
console.log(JSON.stringify(inKeys));

console.log(JSON.stringify(Object.getOwnPropertyDescriptor(o, "hidden")));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(o, "shown")));
const acc = Object.getOwnPropertyDescriptor(o, "computed");
console.log(typeof acc.get, acc.set, acc.enumerable, acc.configurable);
console.log(Object.getOwnPropertyDescriptor(o, "missing"));

console.log(o.propertyIsEnumerable("hidden"), o.propertyIsEnumerable("shown"));
console.log(Object.prototype.hasOwnProperty.call(o, "hidden"), Object.hasOwn(o, "plain"));

// An omitted flag defaults to false (ToPropertyDescriptor), unlike assignment.
const d = {};
Object.defineProperty(d, "x", { value: 7 });
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(d, "x")), JSON.stringify(Object.keys(d)));

console.log(JSON.stringify(Object.getOwnPropertyDescriptors({ a: 1, b: "two" })));
