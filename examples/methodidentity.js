// An ECMAScript intrinsic is ONE function object, shared by every instance.
// Reading a method off an instance used to mint a fresh thunk bound to that
// instance, so every comparison below answered false.
const pairs = [
  ["arr.push", [1].push, Array.prototype.push],
  ["arr.map", [1].map, Array.prototype.map],
  ["str.slice", "a".slice, String.prototype.slice],
  ["num.toFixed", (1).toFixed, Number.prototype.toFixed],
  ["map.get", new Map().get, Map.prototype.get],
  ["set.add", new Set().add, Set.prototype.add],
  ["promise.then", Promise.resolve().then, Promise.prototype.then],
  ["regexp.test", /a/.test, RegExp.prototype.test],
  ["date.getTime", new Date().getTime, Date.prototype.getTime],
  ["obj.hasOwn", ({}).hasOwnProperty, Object.prototype.hasOwnProperty],
  ["fn.call", (function () {}).call, Function.prototype.call],
];
for (const [n, a, b] of pairs) console.log(n.padEnd(13), a === b);
// Two different instances hand out the same function, too.
console.log("instances   ", [1].push === [2].push, "a".slice === "b".slice);
// The shared function still reports its own name and arity, and still works
// through an explicit receiver.
console.log("meta        ", [1].push.name, [1].push.length, "a".slice.name);
console.log("call/apply  ", [].push.apply([1], [2]), [].slice.call("abc", 1).join(""),
  Object.prototype.toString.call(1));

// Because it is shared, a DETACHED method has no receiver at all, and node
// throws. It used to keep quietly working on whatever object it was read off.
const detached = [
  ["Array.push", Array.prototype.push, [1]],
  ["Array.map", Array.prototype.map, [(x) => x]],
  ["Array.join", Array.prototype.join, []],
  ["String.slice", String.prototype.slice, [1]],
  ["String.toUpperCase", String.prototype.toUpperCase, []],
  ["String.trimStart", String.prototype.trimStart, []],
  ["String.valueOf", String.prototype.valueOf, []],
  ["Object.hasOwnProperty", Object.prototype.hasOwnProperty, ["a"]],
  ["Object.valueOf", Object.prototype.valueOf, []],
  ["Object.toLocaleString", Object.prototype.toLocaleString, []],
  ["Number.toFixed", Number.prototype.toFixed, [2]],
  ["Boolean.valueOf", Boolean.prototype.valueOf, []],
  ["Symbol.valueOf", Symbol.prototype.valueOf, []],
  ["Function.call", Function.prototype.call, []],
  ["Function.bind", Function.prototype.bind, []],
  ["Map.get", Map.prototype.get, [1]],
  ["Set.keys", Set.prototype.keys, []],
  ["WeakMap.get", WeakMap.prototype.get, [{}]],
  ["WeakRef.deref", WeakRef.prototype.deref, []],
  ["RegExp.test", RegExp.prototype.test, ["a"]],
  ["Promise.then", Promise.prototype.then, [() => {}]],
  ["Promise.catch", Promise.prototype.catch, [() => {}]],
  ["Promise.finally", Promise.prototype.finally, [() => {}]],
  ["Date.getTime", Date.prototype.getTime, []],
  ["Date.toISOString", Date.prototype.toISOString, []],
  ["Date.toGMTString", Date.prototype.toGMTString, []],
  ["Date.toJSON", Date.prototype.toJSON, []],
  ["ArrayBuffer.slice", ArrayBuffer.prototype.slice, []],
  ["DataView.getInt8", DataView.prototype.getInt8, [0]],
  ["URLSearchParams.get", URLSearchParams.prototype.get, ["a"]],
];
// `Object.prototype.toString` is the one that ACCEPTS a nullish receiver.
console.log("accepts      ", Object.prototype.toString());
for (const [n, f, a] of detached) {
  try { f(...a); console.log(n.padEnd(21), "NO THROW"); }
  catch (e) { console.log(n.padEnd(21), e.constructor.name + ": " + e.message); }
}
// A `null` receiver is named as `null`, not as `undefined`.
for (const [n, f, a] of detached.slice(0, 4).concat([["Map.get", Map.prototype.get, [1]]])) {
  try { f.call(null, ...a); console.log(("null " + n).padEnd(21), "NO THROW"); }
  catch (e) { console.log(("null " + n).padEnd(21), e.message); }
}
