// `JSON.rawJSON` and the reviver's `context.source` (the JSON-source-text
// proposal). Neither existed: `JSON.rawJSON` was a TypeError, and the reviver
// was called with two arguments, so `function (k, v, ctx) { ctx.source }` died
// on `Cannot read properties of undefined`.

// A marker object whose text `JSON.stringify` emits VERBATIM — which is how a
// number too wide for a double survives a round trip.
const big = JSON.rawJSON("12345678901234567890");
console.log("shape    ", typeof big, Object.prototype.toString.call(big),
  JSON.stringify(Object.keys(big)), big.rawJSON);
console.log("frozen   ", Object.getPrototypeOf(big), Object.isFrozen(big),
  JSON.stringify(Object.getOwnPropertyDescriptor(big, "rawJSON")));
console.log("emitted  ", JSON.stringify({ big }), JSON.stringify([big]), JSON.stringify(big));
console.log("roundtrip", JSON.parse(JSON.stringify({ n: big }), (k, v, c) => c.source ?? v).n);
// It is a BRAND, not a shape: a hand-built object with the same property is not
// one, and neither is a copy of the real thing.
console.log("brand    ", JSON.isRawJSON(big), JSON.isRawJSON({ rawJSON: "1" }),
  JSON.isRawJSON({ ...big }), JSON.isRawJSON(1), JSON.isRawJSON(null));
// It survives the replacer, the key filter and indentation, at any depth.
console.log("replacer ", JSON.stringify({ a: 1 }, (k, v) => (k === "a" ? JSON.rawJSON("99") : v)));
console.log("filter   ", JSON.stringify({ a: big, b: 2 }, ["a"]));
console.log("nested   ", JSON.stringify({ o: { p: [big, big] } }));
console.log("indent   ", JSON.stringify({ a: big }, null, 2).replace(/\n/g, "|"));
console.log("kinds    ", JSON.stringify(JSON.rawJSON("1e400")),
  JSON.stringify({ a: JSON.rawJSON("null") }), JSON.stringify(JSON.rawJSON('"s"')));

// Validation is NOT "does JSON.parse accept it". Node reports the parse error
// for a broken literal or a leading-whitespace one, and its own invalid-value
// message for empty input or anything left over — so `" 1"` and `"1 "` fail
// differently even though `JSON.parse` accepts both.
for (const text of ["", " ", " 1", "1 ", "\t1", "1\n", " 1 ", "1", "-1.5e3",
  "true", "false", "null", '"a"', '"a" ', "1,2", "1 2", "nul", "NaN",
  "Infinity", "+1", ".5", "01", "0x1", "{}", "[1]"]) {
  try { console.log("ok       ", JSON.stringify(text), "->", JSON.stringify(JSON.rawJSON(text).rawJSON)); }
  catch (e) { console.log("throw    ", JSON.stringify(text), "->", e.constructor.name + ": " + e.message); }
}
console.log("coerced  ", JSON.rawJSON(1).rawJSON, JSON.rawJSON(true).rawJSON);
console.log("arity    ", JSON.rawJSON.length, JSON.rawJSON.name, JSON.isRawJSON.length);

// The reviver's third argument: `{ source }` carrying the exact source text for
// a PRIMITIVE, and an empty object for an array or an object.
const seen = [];
JSON.parse('{"a":[1,"s",true,null,2.5e3],"b":{"c":-0.5}}', function (k, v, ctx) {
  seen.push(JSON.stringify(k) + " " + (Array.isArray(v) ? "[array]" : typeof v) +
    " keys=" + JSON.stringify(Object.keys(ctx)) + " src=" + JSON.stringify(ctx.source));
  return v;
});
console.log(seen.join("\n"));
console.log("argc     ", JSON.parse("1", function () { return arguments.length; }));
console.log("escapes  ", JSON.parse('"\\u0041"', (k, v, c) => c.source));
// A reviver that DROPS a key must not disturb the source cursor for the rest.
console.log("drop     ", JSON.stringify(JSON.parse('{"a":{"x":1},"b":3}',
  function (k, v, c) { return k === "x" ? undefined : v; })));
