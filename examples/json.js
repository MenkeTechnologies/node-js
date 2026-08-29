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
