// querystring: parse / stringify / escape / unescape.
const qs = require("querystring");

const parsed = qs.parse("foo=bar&baz=qux&baz=quux&corge=");
console.log(JSON.stringify(parsed));

console.log(qs.stringify({ foo: "bar", baz: ["qux", "quux"], corge: "" }));
console.log(qs.stringify({ a: "1", b: "2" }, ";", ":"));

console.log(qs.escape("hello world & friends=1"));
console.log(qs.unescape("hello%20world%20%26%20friends%3D1"));

const nested = qs.parse("a=1&a=2&a=3&b=hello");
console.log(Array.isArray(nested.a), nested.a.join("|"), nested.b);
console.log(qs.stringify({ w: "with space", s: "a/b?c" }));
