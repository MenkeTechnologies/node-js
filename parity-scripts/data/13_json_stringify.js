// JSON.stringify: nested, indent, arrays, nulls.
const obj = {
  name: "test",
  count: 42,
  active: true,
  nested: { a: [1, 2, 3], b: null },
  list: [{ x: 1 }, { x: 2 }],
};
console.log(JSON.stringify(obj));
console.log(JSON.stringify(obj, null, 2));
console.log(JSON.stringify(obj, null, "\t"));

console.log(JSON.stringify([1, "two", true, null, [3, 4]]));
console.log(JSON.stringify({ a: undefined, b: 1, c: null }));  // undefined dropped
console.log(JSON.stringify([undefined, function () {}, 1]));    // nulls
console.log(JSON.stringify({ n: NaN, i: Infinity }));           // nulls
console.log(JSON.stringify("string with \"quotes\" and \n newline"));
console.log(JSON.stringify(3.14));
console.log(JSON.stringify(null));
console.log(JSON.stringify({ nested: { deep: { deeper: 1 } } }, null, 2));

// replacer array (key filter)
console.log(JSON.stringify(obj, ["name", "count"]));
console.log(JSON.stringify({ z: 1, a: 2, m: 3 }));  // insertion order
