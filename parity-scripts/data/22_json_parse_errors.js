// JSON.parse failure messages — V8 emits a family of distinct SyntaxErrors, and
// packages (body-parser among them) surface the text verbatim to users.
const cases = [
  "NOTJSON", "{", '{"a":}', "[1,2,", "tru", '{"a":1}x', '{"a" 1}', "undefined",
  "'x'", "01", "00", "-01", "NaN", "Infinity", "-Infinity", "undef", "nul",
  "nan", "foo", "u", "undefinedx", "[undefined]", "truex", '{"a":undefined}',
  '"abc', "1 2", "[1 2]", '{"a":1,}', "[1,]", "a".repeat(5), "a".repeat(20),
  "a".repeat(21), "a".repeat(30), "[" + "1,".repeat(10) + "@]",
  "[" + "1,".repeat(1) + "@]", "[" + "1,".repeat(5) + "@]",
  '{"a"::1}', "{,}", "[,]", '{"a":1,,"b":2}', "12e", "1e", "1e+", "1.2.3", "-",
  "--1", "1.", ".5", "0x10", '{"a":1}}', "[[]", "[1", "1,", '{"a"}', "[1,,2]",
  '"unterminated', '"raw\u0009tab"', '{"nested":{"deep":{"x":1}}}extra', '{\n "a" 1}', "[\n1 2]",
  '"a"b', "[]]", "{}{}", '{"a":1} 2', '[1] "x"', '5"x"', '5 "x"',
  '{"a":"b"}"c"', "[0,1]0", "", "   ", "\t\n",
];
for (const c of cases) {
  try {
    console.log("OK", JSON.stringify(JSON.parse(c)));
  } catch (e) {
    console.log(e.name + ": " + e.message);
  }
}
// Inputs that must still parse.
const ok = ['{"a":1}  ', '  {"a":1}', "[]", "{}", "null", "true", "1e5", "-0",
  "0.5", '{"k":"v","x":[1,2,{"y":null}]}', "[[[[1]]]]", '{"a":1}\n\n',
  '\n{"a":1}', '"é"', '"tab\\there"', '"\\u00e9"', "1e999", '{"__proto__":1}'];
for (const c of ok) console.log(JSON.stringify(JSON.parse(c)));
