// Reflect: what each method validates and in which order. A refused
// definition is `false` where Object.defineProperty throws, yet a malformed
// descriptor still throws; apply/construct check the target before the list.
const P = (f) => { try { const v = f(); return typeof v === "bigint" ? v + "n" : String(v); } catch (e) { return e.name + ": " + e.message; } };
const prims = [1, "s", null, undefined, Symbol("q"), true];
for (const t of prims) {
  console.log(String(typeof t), [
    () => Reflect.getPrototypeOf(t), () => Reflect.setPrototypeOf(t, null), () => Reflect.isExtensible(t),
    () => Reflect.preventExtensions(t), () => Reflect.ownKeys(t), () => Reflect.get(t, "a"),
    () => Reflect.set(t, "a", 1), () => Reflect.has(t, "a"), () => Reflect.deleteProperty(t, "a"),
    () => Reflect.defineProperty(t, "a", {}), () => Reflect.getOwnPropertyDescriptor(t, "a"),
  ].map(P).join(" | "));
}
const descs = [{ value: 1 }, { get() {} }, { get: 1 }, { set: "x" }, { get() {}, value: 1 }, { set() {}, writable: true }, 1, "d", null, undefined, []];
for (const d of descs) {
  console.log(P(() => Reflect.defineProperty({}, "k", d)), "/", P(() => Object.defineProperty({}, "k", d)));
}
console.log(P(() => Reflect.defineProperty(Object.freeze({}), "k", { value: 1 })));
console.log(P(() => Reflect.defineProperty(Object.preventExtensions({}), "k", {})));
console.log(P(() => Object.getOwnPropertyDescriptor(Object.defineProperty({}, "k", {}), "k") && JSON.stringify(Object.getOwnPropertyDescriptor(Object.defineProperty({}, "k", {}), "k"))));
for (const p of [null, {}, 1, undefined, "s"]) console.log(P(() => Reflect.setPrototypeOf({}, p)));
for (const f of [Math.max, 1, undefined, () => 7, class C {}, Symbol]) {
  for (const l of [[1, 3], 1, undefined, null]) console.log(P(() => Reflect.apply(f, null, l)));
}
for (const t of [Date, function () {}, () => {}, 1, class C {}, async function g() {}, Math.max, Map, function* gen() {}]) {
  for (const n of [Object, () => {}, 1, undefined]) console.log(P(() => typeof Reflect.construct(t, [], n)));
}
