// Three standard statics that did not exist here at all.

// `Promise.try(fn, ...args)` - call `fn` and settle with what it does, so a
// SYNCHRONOUS throw becomes a rejection. It is the shape `Promise.resolve()
// .then(fn)` is written for, without that form's extra tick.
console.log("try-meta  ", typeof Promise.try, Promise.try.length, Promise.try.name);
Promise.try(() => 1).then((v) => console.log("try-value ", v));
Promise.try((a, b) => a + b, 1, 2).then((v) => console.log("try-args  ", v));
Promise.try(async () => 2).then((v) => console.log("try-async ", v));
// The REJECTION carries the thrown object itself, not a rebuild of its text.
Promise.try(() => { throw new TypeError("t"); })
  .catch((e) => console.log("try-throw ", e.constructor.name, e.message));
Promise.try(() => Promise.reject(new RangeError("r")))
  .catch((e) => console.log("try-reject", e.constructor.name, e.message));
// A non-callable argument REJECTS rather than throwing, so the surrounding
// `try` never sees it, and the message names the argument's TYPE as well as
// its value - which the ordinary call-site message does not.
Promise.all([5, "s", undefined, null, {}, true, Symbol("x"), 1n, [1]]
  .map((v) => Promise.try(v).catch((e) => e.constructor.name + ": " + e.message)))
  .then((all) => all.forEach((m) => console.log("try-bad   ", m)));

// `RegExp.escape(s)` - a pattern matching `s` literally. The rule is not just
// "backslash the syntax characters": a LEADING ASCII alphanumeric becomes
// `\xNN` so the result can follow a `\` or a `{` without running into it, and
// the punctuation that is meaningful inside a class or a group name is escaped
// by code point. The whitespace set is ECMAScript's, not Unicode's: U+0085 and
// U+001C-U+001F are whitespace to Unicode and are left ALONE here, so the check
// cannot be a general Unicode one.
console.log("esc-meta  ", typeof RegExp.escape, RegExp.escape.length, RegExp.escape.name);
const samples = ["a", "a.b", "^$\\.*+?()[]{}|/", "0abc", "_x", " sp", "\n\t", "",
  "ab-cd", "\u03a9", "\u{1F600}", "a\u{1F600}", "x y", "Z\u3000", "\u00a0",
  "\u2028", "\u200b", "\u180e", "\u007f", "\u2009", "\u0085", "\u001c"];
for (const s of samples) {
  console.log("esc", JSON.stringify(s), "->", JSON.stringify(RegExp.escape(s)));
}
const literal = "a.b*c";
console.log("roundtrip ", new RegExp(RegExp.escape(literal)).test(literal),
  new RegExp(RegExp.escape(literal)).test("axbxc"));
try { RegExp.escape(1); } catch (e) { console.log("esc-bad   ", e.constructor.name, e.message); }
try { RegExp.escape(); } catch (e) { console.log("esc-noarg ", e.message); }

// `Error.isError(v)` - a brand check, so inheriting from `Error.prototype` is
// not enough.
class Subclass extends Error {}
console.log("isError   ", typeof Error.isError, Error.isError.length, Error.isError.name);
console.log("isError-y ", Error.isError(new Error()), Error.isError(new TypeError()),
  Error.isError(new Subclass()), Error.isError(new DOMException("m")));
console.log("isError-n ", Error.isError({}), Error.isError(Object.create(Error.prototype)),
  Error.isError(null), Error.isError(Error), Error.isError("Error: x"));
