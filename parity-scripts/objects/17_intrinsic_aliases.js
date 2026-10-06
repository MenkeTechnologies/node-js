// A legacy or symbol-keyed alias is the SAME function object as the method it
// aliases, so identity comparisons and the alias's own name/length agree.
const TA = Object.getPrototypeOf(Uint8Array.prototype);
console.log(
  Array.prototype[Symbol.iterator] === Array.prototype.values,
  [][Symbol.iterator] === [].values,
  Map.prototype[Symbol.iterator] === Map.prototype.entries,
  Set.prototype[Symbol.iterator] === Set.prototype.values,
  Set.prototype.keys === Set.prototype.values,
  new Set().keys === Set.prototype.values,
  TA[Symbol.iterator] === TA.values,
  URLSearchParams.prototype[Symbol.iterator] === URLSearchParams.prototype.entries,
  Date.prototype.toGMTString === Date.prototype.toUTCString,
  new Date(0).toGMTString === Date.prototype.toUTCString,
);
// The Annex B string trims, and that they stay distinct from their neighbours.
console.log(
  String.prototype.trimLeft === String.prototype.trimStart,
  String.prototype.trimRight === String.prototype.trimEnd,
  String.prototype.trimLeft === String.prototype.trimEnd,
  String.prototype[Symbol.iterator] === String.prototype.values,
);
console.log(String.prototype.trimLeft.name, String.prototype.trimRight.name,
  String.prototype.trimLeft.length, Set.prototype.keys.name, Date.prototype.toGMTString.name);
console.log(JSON.stringify(" \t x \n".trimLeft()), JSON.stringify(" \t x \n".trimRight()),
  JSON.stringify(String.prototype.trimLeft.call("﻿ y")));
console.log(Object.getOwnPropertyNames(String.prototype).filter((k) => k.startsWith("trim")));
// A Map keyed by functions sees one entry per intrinsic, not per alias.
const seen = new Map([[Set.prototype.values, "values"]]);
console.log(seen.get(Set.prototype.keys), seen.has(Set.prototype[Symbol.iterator]));
