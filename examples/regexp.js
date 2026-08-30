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

// `RegExp.prototype[@@split]` (22.2.6.14). Every one of these is a position
// where a separator can match empty, and each was wrong: the scan used to
// append a trailing "" that the spec's `q < size` loop bound never reaches.
console.log(JSON.stringify("ab".split(/(?:)/)), JSON.stringify("".split(/(?:)/)));
console.log(JSON.stringify("ab".split(/x*/)), JSON.stringify("xab".split(/x*/)), JSON.stringify("abx".split(/x*/)));
// A capture group participates in the output; an empty one is not a piece.
console.log(JSON.stringify("ab".split(/()/)), JSON.stringify("a1b".split(/(\d)/)), JSON.stringify("ab".split(/(x)?/)));
// End-anchored zero-width separators match at `size`, which the loop never
// scans to — `/$/` and `/\b/` had been growing an extra "" apiece.
console.log(JSON.stringify("ab".split(/$/)), JSON.stringify("foo bar".split(/\b/)), JSON.stringify("a1b2".split(/(?<=\d)/)));
// A real separator at the end does still leave a trailing empty piece.
console.log(JSON.stringify("a,b,".split(/,/)), JSON.stringify(",".split(/,/)));
// `limit` counts appended elements, captures included, and 0 yields nothing.
console.log(JSON.stringify("abc".split(/(?:)/, 2)), JSON.stringify("a1b2c".split(/(\d)/, 3)), JSON.stringify("abc".split(/b/, 0)));
// Splitting on the whole subject leaves the two empty sides.
console.log(JSON.stringify("abc".split(/abc/)), JSON.stringify("aXXb".split(/X/)));

// A global regexp carries `lastIndex` as mutable state, and the methods that
// consume the WHOLE string reset it. `replace` and `match` were leaving it
// wherever the caller had put it, so a shared `/…/g` skipped the front of the
// string on its next use — the classic reason a regexp "stops matching".
const shared = /a/g;
shared.lastIndex = 3;
console.log("repl-idx", "aaa".replace(shared, "x"), shared.lastIndex);
const scan = /a/g;
scan.lastIndex = 2;
console.log("match-idx", JSON.stringify("aaa".match(scan)), scan.lastIndex);
// A NON-global replace leaves it alone, and `matchAll` leaves its own alone.
const once = /a/;
once.lastIndex = 3;
"aaa".replace(once, "x");
const all = /a/g;
all.lastIndex = 2;
[..."aaa".matchAll(all)];
console.log("kept    ", once.lastIndex, all.lastIndex);
// Which is what makes a shared global regexp usable twice in a row.
const reused = /a/g;
console.log("reuse   ", "aaa".replace(reused, "x"), reused.test("aaa"), reused.lastIndex);

// `matchAll` cannot produce every match without `g`, so a non-global regexp is
// a TypeError (22.1.3.14) — the same rule `replaceAll` enforces. It used to
// return just the first match.
try { [..."ab".matchAll(/a/)]; } catch (e) { console.log("matchAll", e.constructor.name, e.message); }
console.log("global  ", [..."aa".matchAll(/a/g)].length);

// The `d` flag (match indices, 22.2.7.8 MakeMatchIndicesIndexPairArray). The
// flag itself parsed — `flags` and `hasIndices` both read correctly — but no
// match ever carried `.indices`, so the whole point of it was missing.
const show = (m) => JSON.stringify(m && m.indices);
console.log("flags   ", /a/d.flags, /a/d.hasIndices, /a/.hasIndices, String(/a/dg), new RegExp("a", "dgimsuy").flags);
console.log("basic   ", show(/b/d.exec("abc")), show(/(a)(b)/d.exec("xab")));
// A group that did not participate reports `undefined`, not a pair.
console.log("optional", show(/(a)?(b)/d.exec("b")));
// Named groups mirror into `indices.groups`, which is null-prototype like
// `groups` itself.
const dated = /(?<y>\d{4})-(?<mo>\d{2})/d.exec("on 2024-05");
console.log("named   ", show(dated), JSON.stringify(dated.indices.groups),
  Object.getPrototypeOf(dated.indices.groups));
// Offsets are UTF-16 indices, the same units `.index` uses.
console.log("units   ", show(/b/d.exec("ab")), /b/d.exec("ab").index);
// Without the flag there are no indices at all.
console.log("absent  ", String(/(a)/.exec("a").indices));
// Every entry point that produces a match array carries them.
console.log("paths   ", show("xay".match(/a/d)),
  [...("aa".matchAll(/a/dg))].map(show).join(" "));
const sticky = /a/dy;
sticky.lastIndex = 1;
console.log("sticky  ", show(sticky.exec("ba")));
