// Constructors with no [[Call]] behaviour. V8 words the refusal by the kind of
// constructor; a constructor that is also a function (String, Number, Error,
// Array, Date …) answers a plain call.
const names = ["Map", "Set", "WeakMap", "WeakSet", "WeakRef", "Promise", "ArrayBuffer", "DataView",
  "Uint8Array", "Float64Array", "BigInt64Array", "FinalizationRegistry", "TextEncoder", "TextDecoder",
  "URL", "URLSearchParams", "AbortController", "Iterator", "Proxy", "Symbol", "BigInt",
  "Error", "TypeError", "AggregateError", "Array", "Object", "Function", "RegExp", "Boolean", "Number",
  "String", "Date"];
for (const n of names) {
  let r;
  try { const v = globalThis[n](); r = "ok " + typeof v; } catch (e) { r = e.constructor.name + ": " + e.message; }
  console.log(n, r);
}
