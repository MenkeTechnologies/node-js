// OrdinaryOwnPropertyKeys: array-index keys first (ascending), then string
// keys in creation order, then symbols — for functions, classes and class
// prototypes too. A symbol-keyed accessor is a symbol key, never a string.
const o = {}; o.b = 1; o[2] = 1; o.a = 1; o[1] = 1; o["01"] = 1; o[4294967295] = 1; o[4294967294] = 1;
console.log(Object.keys(o), JSON.stringify(o), Reflect.ownKeys(o));
for (const k in o) process.stdout.write(k + " "); console.log();
class X { get [Symbol.split]() { return 1 } 2() {} a() {} 1() {} static 2() {} static b() {} static 1() {} }
console.log(Object.getOwnPropertyNames(X.prototype), Object.getOwnPropertyNames(X), Reflect.ownKeys(X.prototype));
function f() {} f.z = 1; f[3] = 1; f[0] = 1; console.log(Object.keys(f), Reflect.ownKeys(f));
const lit = { get [Symbol.split]() { return 1 }, b: 1, 2: 1, get 1() { return 1 } };
console.log(Object.getOwnPropertyNames(lit), Reflect.ownKeys(lit), Object.keys(lit));
const od = {}; Object.defineProperty(od, Symbol.iterator, { get() { return 1 }, enumerable: true }); od.x = 1;
console.log(Object.getOwnPropertyNames(od), Object.keys(od), od);
console.log(Object.assign({}, { b: 1, 1: 2 }), { ...{ z: 1, 0: 1 } }, Object.entries({ y: 1, 5: 2, 3: 1 }));
