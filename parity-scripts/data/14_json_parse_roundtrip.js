// JSON.parse and round-trips.
const src = '{"a":1,"b":[2,3,4],"c":{"d":true,"e":null},"f":"text"}';
const parsed = JSON.parse(src);
console.log(parsed.a, parsed.b[1], parsed.c.d, parsed.c.e, parsed.f);
console.log(JSON.stringify(parsed) === src);

console.log(JSON.parse("42"), JSON.parse("true"), JSON.parse("null"));
console.log(JSON.parse('"hello"'));
console.log(JSON.parse("[1,2,3]").reduce((a, b) => a + b));
console.log(JSON.parse('{"x":1.5e2}').x);
console.log(JSON.parse('["\\u0041","\\t"]'));

// round-trip a structure
const data = { ids: [1, 2, 3], meta: { total: 3, tags: ["a", "b"] } };
const round = JSON.parse(JSON.stringify(data));
console.log(JSON.stringify(round));
console.log(round.ids.length, round.meta.total);

// reviver function
const revived = JSON.parse('{"n":5}', (k, v) => (typeof v === "number" ? v * 10 : v));
console.log(revived.n);

try {
  JSON.parse("{bad}");
} catch (e) {
  console.log(e.constructor.name);
}
