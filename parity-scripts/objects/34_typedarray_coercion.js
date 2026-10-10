// Typed-array element coercion (ToInt8 … ToUint32, ToUint8Clamp), the
// non-finite cases, array-like construction, ordering with NaN and -0, and the
// indexed own property's attributes.
const P = (f) => { try { const v = f(); return Array.isArray(v) ? v.map((x) => Object.is(x, -0) ? "-0" : x).join() : String(v); } catch (e) { return e.name + ": " + e.message; } };
const kinds = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "Float32Array", "Float64Array"];
const vals = [0.5, 1.5, 2.5, 3.5, 254.5, 255.5, -0.5, 255, 256, 257, -1, -129, 65535, 65536, 2 ** 31, 2 ** 32, 2 ** 32 + 7, 1e10, 1e20, -1e20, 2 ** 53, NaN, Infinity, -Infinity, -0, 1.1, 16777217, 5e-46, 3.4e39];
for (const k of kinds) console.log(k, P(() => Array.from(globalThis[k].from(vals))));
for (const k of kinds) console.log(k, P(() => Array.from(new globalThis[k]({ length: 3, 0: 7, 1: "9", 2: null }))));
for (const k of kinds) console.log(k, P(() => Array.from(new globalThis[k]([3, NaN, -0, 0, -Infinity, 1, Infinity, 2.5]).sort())));
console.log(P(() => new Uint8Array([1n])), P(() => new BigInt64Array([1])), P(() => new BigInt64Array([1n, true, "7"])));
const u = new Uint8Array(4);
console.log([delete u[0], delete u[9], delete u.foo, Reflect.deleteProperty(u, 1)].join());
for (const d of [{ value: 7 }, { value: 7, writable: false }, { value: 7, enumerable: false }, { value: 7, configurable: false },
  { value: 7, writable: true, enumerable: true, configurable: true }, { get() {} }, {}]) {
  console.log(P(() => Object.defineProperty(u, 0, d) === u), P(() => Reflect.defineProperty(u, 9, d)), P(() => Reflect.defineProperty(u, 1, d)));
}
console.log(P(() => Uint8Array(1)), P(() => Float64Array(1)), P(() => ArrayBuffer(8)), P(() => DataView(new ArrayBuffer(8))));
