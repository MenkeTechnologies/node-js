// `\uXXXX` and `\u{…}` escapes inside identifiers spell the same name, and a
// string's escapes include the legacy octal ones.
var ab = 3;
var abc = 4;
var \u{61}bcd = 5;
console.log(ab, abc, abcd, ({ a: 1 }).a);
console.log("\101\60\x41A\u{1F600}".length, "\101\60\x41A", "\0".length, "\08".length);
console.log("\1\12\123\1234".split("").map((c) => c.charCodeAt(0)).join());
console.log("\8\9", "\400".length, "a\
b");
