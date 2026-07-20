// Regular expressions: the Rust-regex-supported subset (char classes,
// quantifiers, anchors, groups, alternation, \d\w\s, flags g/i/m). node-js
// rejects backreferences/lookaround rather than mis-executing them.

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
