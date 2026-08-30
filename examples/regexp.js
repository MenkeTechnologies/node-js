// Regular expressions: char classes, quantifiers, anchors, groups,
// alternation, \d\w\s, the g/i/m flags, and the lookaround and backreference
// forms — all six of which run here and agree with node.

// test / exec.
console.log(/\d+/.test("abc123"), /^foo/.test("foobar"), /z/.test("abc"));
console.log(/(\w)(\w)/.exec("ab")[1], /(\w)(\w)/.exec("ab")[2]);
console.log(/\d+/.exec("year 2024 end").index);

// Properties.
console.log(/a.c/gi.source, /a.c/gi.flags, /x/g.global, /x/i.ignoreCase);

// String methods with a regex.
console.log("a1b2c3".replace(/\d/g, "#"));
console.log("2024-01-15".replace(/(\d+)-(\d+)-(\d+)/, "$3/$2/$1"));
console.log("hello world foo".split(/\s+/));
console.log("a1b2c3".match(/\d/g));
console.log("foobar".search(/bar/));
console.log([..."a1b2c3".matchAll(/\d/g)].map((m) => m[0]));

// Named capture groups.
console.log("age 36".match(/(?<n>\d+)/).groups.n);

// The `g` flag advances lastIndex across calls.
const re = /\d/g;
console.log(re.test("a1b2"), re.lastIndex, re.test("a1b2"), re.lastIndex);

// A case-insensitive, multi-line match.
console.log("Foo\nbar\nBAZ".match(/^b\w+/gim));

// Constructing via new RegExp.
console.log(new RegExp("\\d{3}").test("ab123"), new RegExp("x", "gi").flags);

// Lookaround and backreferences.
console.log("foo1".match(/foo(?=\d)/)[0], "fooX".match(/foo(?!\d)/)[0]);
console.log("a$42".match(/(?<=\$)\d+/)[0], "a42".match(/(?<!\$)\d+/)[0]);
console.log("abab".match(/(ab)\1/)[0], "xx".match(/(?<c>x)\k<c>/)[0]);

// `source` is escaped (22.2.6.13 EscapeRegExpPattern) so that `/` + source +
// `/` re-parses as a literal matching the same thing. A `/` left unescaped
// closed the literal early — `String(new RegExp("/"))` read back as `///`,
// which is not parseable — and a literal newline, which cannot appear in a
// literal at all, came through raw instead of as the two characters `\n`. A
// `/` inside a character class needs no escape and gets none.
console.log(new RegExp("/").source, new RegExp("a/b").source, new RegExp("[/]").source);
console.log(String(new RegExp("/")), JSON.stringify(new RegExp("\n").source));
console.log(new RegExp("").source, new RegExp(new RegExp("a/b").source).test("a/b"));
