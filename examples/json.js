// JSON round-trips: key order, spacing, replacer/reviver, and the values that
// have no JSON form.
const o = { b: 2, a: 1, nested: { list: [1, "two", null, true] } };
console.log(JSON.stringify(o));
console.log(JSON.stringify(o, null, 2));
console.log(JSON.stringify(o, ["a", "b"]));
console.log(JSON.stringify(o, (k, v) => (typeof v === "number" ? v * 10 : v)));
console.log(JSON.stringify([undefined, function () {}, Symbol("s")]));
console.log(JSON.stringify({ u: undefined, f: function () {} }));
console.log(JSON.stringify("he\"llo\n"), JSON.stringify(NaN), JSON.stringify(Infinity));
const parsed = JSON.parse('{"x":1,"y":[2,3],"z":{"k":"v"}}');
console.log(parsed.x, parsed.y, parsed.z.k);
console.log(JSON.parse("[1,2,3]", (k, v) => (typeof v === "number" ? v + 1 : v)));
try { JSON.parse("{bad}"); } catch (e) { console.log(e.name); }
console.log(JSON.stringify({ toJSON() { return "custom"; } }));

// A `JSON.parse` reviver runs with the HOLDER as `this` — the object or array
// the key lives in (25.5.1.1) — so it can reach its siblings. It was being
// called with no receiver, making `this` undefined and that impossible.
console.log("sibling ", JSON.stringify(JSON.parse('{"a":1,"b":2}', function (k, v) {
  return k === "b" ? v + this.a : v;
})));
console.log("nested  ", JSON.stringify(JSON.parse('{"o":{"x":1,"y":2}}', function (k, v) {
  return k === "y" ? v + this.x : v;
})));
console.log("array   ", JSON.parse("[1,2]", function (k, v) {
  return k === "1" ? (Array.isArray(this) ? "arr" : "not") : v;
})[1]);
// At the top level the holder is a fresh `{ "": value }` wrapper.
console.log("root    ", JSON.parse("5", function (k, v) {
  return k === "" ? [JSON.stringify(k), typeof this, Object.keys(this).length].join("|") : v;
}));
// The walk is still bottom-up and a returned undefined still drops the key.
const order = [];
JSON.parse('{"a":{"b":1},"c":2}', function (k, v) { order.push(k); return v; });
console.log("order   ", order.join(","));
console.log("dropped ", JSON.stringify(JSON.parse('{"a":1,"b":2}', (k, v) => (k === "a" ? undefined : v))));
