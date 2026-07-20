// Spread operator for arrays, objects, and calls.
const a = [1, 2, 3];
const b = [4, 5, 6];
const combined = [...a, ...b];
console.log("combined:", combined.join(","));

const withMiddle = [...a, 0, ...b];
console.log("withMiddle:", withMiddle.join(","));

// Object spread and override.
const base = { x: 1, y: 2, z: 3 };
const patched = { ...base, y: 20, w: 40 };
console.log(JSON.stringify(patched));

// Spread into function call.
function sum3(p, q, r) {
  return p + q + r;
}
console.log("sum3:", sum3(...a));

// Math.max with spread.
console.log("max:", Math.max(...combined));

// Copy and dedupe via Set.
const dupes = [1, 1, 2, 3, 3, 3, 4];
const unique = [...new Set(dupes)];
console.log("unique:", unique.join(","));

// Clone a nested structure (shallow) and mutate copy.
const orig = { list: [1, 2], name: "orig" };
const copy = { ...orig, name: "copy" };
console.log(copy.name, orig.name);

// Spread string into chars.
console.log([..."hello"].reverse().join(""));
