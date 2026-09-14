// A symbol is the one value whose `ToString` and `ToNumber` both THROW (7.1.17
// step 2, 7.1.4 step 2). Every builtin that reached for the infallible
// stringifier instead rendered `Symbol(desc)` into its result and reported
// nothing — a symbol key leaking into text, silently.
//
// Which of the two messages node uses is per ARGUMENT POSITION, not per method:
// `'x'.indexOf(sym)` is the STRING one and `'x'.indexOf('a', sym)` the NUMBER
// one, and `padStart` is the same pair the other way round.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const s = Symbol("d");

// String positions.
show("padStart-str ", () => "x".padStart(3, s));
show("padEnd-str   ", () => "x".padEnd(3, s));
show("concat       ", () => "x".concat(s));
show("startsWith   ", () => "x".startsWith(s));
show("endsWith     ", () => "x".endsWith(s));
show("includes     ", () => "x".includes(s));
show("indexOf      ", () => "x".indexOf(s));
show("lastIndexOf  ", () => "x".lastIndexOf(s));
show("split        ", () => "x".split(s));
show("replace      ", () => "abc".replace("b", s));
show("replaceAll   ", () => "abc".replaceAll("b", s));
show("localeCompare", () => "x".localeCompare(s));
show("normalize    ", () => "x".normalize(s));
show("match        ", () => "x".match(s));
show("search       ", () => "x".search(s));
// Number positions — same method, different argument.
show("padStart-num ", () => "x".padStart(s));
show("indexOf-2nd  ", () => "x".indexOf("a", s));
show("includes-2nd ", () => "x".includes("a", s));
show("split-limit  ", () => "x".split("a", s));
show("substring-2nd", () => "x".substring(0, s));
show("at           ", () => "x".at(s));
show("charAt       ", () => "x".charAt(s));
show("charCodeAt   ", () => "x".charCodeAt(s));
show("codePointAt  ", () => "x".codePointAt(s));
show("repeat       ", () => "x".repeat(s));
show("slice        ", () => "x".slice(s));
// The same conversion outside `String.prototype`.
show("array-join   ", () => [1, 2].join(s));
show("error-message", () => new Error(s).message);
show("typeerror    ", () => new TypeError(s).message);
show("regexp-source", () => new RegExp(s).source);
show("regexp-flags ", () => new RegExp("a", s).flags);
show("date         ", () => new Date(s).getTime());
show("encodeURI    ", () => encodeURI(s));
show("decodeURI    ", () => decodeURI(s));
show("encodeURIComp", () => encodeURIComponent(s));
// …and the conversions a symbol is ALLOWED, which must keep working.
show("String()     ", () => String(s));
show("toString     ", () => s.toString());
show("description  ", () => s.description);
show("property-key ", () => { const o = {}; o[s] = 1; return o[s]; });
show("Boolean      ", () => Boolean(s));
show("typeof       ", () => typeof s);
show("json-value   ", () => [JSON.stringify(s), JSON.stringify({ a: s })]);
show("join-default ", () => [1, 2].join());
show("join-undef   ", () => [1, 2].join(undefined));
