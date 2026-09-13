// The own property names of an intrinsic prototype. `Object.getOwnPropertyNames`
// answered `[]` for every one of them: the members were reachable by name
// through the builtin dispatch but were not enumerable, so anything that walks a
// prototype — feature detection, a mixin copy — found nothing there.
//
// Sorted at every comparison below because the order is the engine's insertion
// order, which this frontend does not reproduce.
const names = (o) => Object.getOwnPropertyNames(o).sort().join(",");
console.log("Map     ", names(Map.prototype));
console.log("Set     ", names(Set.prototype));
console.log("Promise ", names(Promise.prototype));
console.log("WeakMap ", names(WeakMap.prototype));
console.log("WeakRef ", names(WeakRef.prototype));
console.log("Date    ", names(Date.prototype).length, names(Date.prototype).slice(0, 60));
console.log("Array   ", names(Array.prototype).length, names(Array.prototype).slice(0, 60));

// Accessors are in the list too, which is why it cannot be derived from the
// set of prototype FUNCTIONS: `size`, `source` and `byteLength` are not methods.
console.log("accessor", Object.getOwnPropertyNames(Map.prototype).includes("size"),
  Object.getOwnPropertyNames(RegExp.prototype).includes("source"),
  Object.getOwnPropertyNames(ArrayBuffer.prototype).includes("byteLength"));
console.log("RegExp  ", names(RegExp.prototype));
console.log("ArrayBuf", names(ArrayBuffer.prototype));
console.log("DataView", names(DataView.prototype).length);

// An ECMAScript builtin's prototype members are NON-enumerable; a WebIDL
// interface's are plain assigned properties, so they are enumerable. The two
// answers differ, and both are pinned.
console.log("nonenum ", JSON.stringify(Object.keys(Map.prototype)),
  JSON.stringify(Object.keys(Promise.prototype)),
  JSON.stringify(Object.keys(Array.prototype)));
console.log("webidl  ", Object.keys(URLSearchParams.prototype).sort().join(","));
console.log("url     ", Object.keys(URL.prototype).sort().join(","));
console.log("encoder ", Object.keys(TextEncoder.prototype).sort().join(","));

// A member read off the list is the same function a property read gives.
console.log("callable", typeof Map.prototype.get, Map.prototype.get.name,
  Map.prototype.get.length, Object.getOwnPropertyNames(Map.prototype).includes("get"));
