// Regex test/exec/match (Rust-regex-supported subset).
const re = /\d+/g;
console.log(re.test("abc123"));
console.log("a1b22c333".match(/\d+/g));
console.log("hello world".match(/\w+/g));
console.log("  spaced  out ".match(/\S+/g));

const ex = /(\d{4})-(\d{2})-(\d{2})/;
const m = ex.exec("date: 2026-07-20 end");
console.log(m[0], m[1], m[2], m[3]);
console.log(m.index);

console.log(/^[a-z]+$/.test("hello"));
console.log(/^[a-z]+$/.test("Hello"));
console.log(/^\d{3}-\d{4}$/.test("555-1234"));
console.log(/colou?r/.test("color"), /colou?r/.test("colour"));
console.log("cat bat rat".match(/[cbr]at/g));
console.log("a|b|c".split(/\|/));
console.log(/foo|bar|baz/.test("xbarx"));
console.log("AAABBBCCC".match(/A+|B+|C+/g));
console.log("x123y456".match(/[a-z]\d+/g));
console.log(/\bword\b/.test("a word here"));
