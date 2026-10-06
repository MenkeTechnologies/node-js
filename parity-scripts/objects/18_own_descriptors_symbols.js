// Object.getOwnPropertyDescriptors walks [[OwnPropertyKeys]]: string keys
// (integer indices first), then symbol keys — enumerable or not.
const hidden = Symbol("hidden");
const o = { b: 1, 2: "two", get g() { return 3; }, [Symbol.toStringTag]: "Tagged", a: 0 };
Object.defineProperty(o, hidden, { value: 4, enumerable: false });
Object.defineProperty(o, "ne", { value: 5, enumerable: false });
const d = Object.getOwnPropertyDescriptors(o);
console.log(d);
console.log(Reflect.ownKeys(d));
// The clone-with-accessors idiom keeps symbol-keyed members.
const clone = Object.create(Object.getPrototypeOf(o), d);
console.log(String(clone), clone[hidden], clone.g, Reflect.ownKeys(clone));
console.log(Object.getOwnPropertyDescriptors([7]));
console.log(Object.getOwnPropertyDescriptors(class { static [Symbol.iterator]() {} }));
const p = new Proxy({ x: 1, [Symbol.for("y")]: 2 }, {});
console.log(Object.getOwnPropertyDescriptors(p));
