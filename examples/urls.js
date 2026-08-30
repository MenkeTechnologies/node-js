// WHATWG URL parsing and the base64 globals. Nothing in the corpus reached
// either, and the parser was doing no percent-encoding at all: the components
// came back with their spaces and control characters intact, so `href` was not
// a valid URL and did not round-trip.

// Each component has its own percent-encode set. These were measured against
// node by feeding every ASCII character through each position, because the
// spec's sets and what a parser emits differ around the delimiters.
const u = new URL("https://a.b/a b?c=d e#f g");
console.log("encode  ", u.pathname, u.search, u.hash);
console.log("href    ", u.href);
console.log("roundtrip", new URL(u.href).href === u.href);
// path encodes " \"<>^`{}" — but not | ! $ & ' ( ) * + , ; : @ ~ - _ .
console.log("path    ", new URL('https://a.b/x"<>^`{}y').pathname);
console.log("path-ok ", new URL("https://a.b/x|!$&'()*+,;:@~-_.y").pathname);
// query adds ' and drops ^ ` { }
console.log("query   ", new URL("https://a.b/?x\"'<>^`{}y").search);
// fragment keeps ' and { } but encodes `
console.log("hash    ", new URL("https://a.b/#x\"'<>`{}y").hash);
// userinfo is the widest set of the four
console.log("user    ", new URL("https://x <>;=@[]^`{|}y:p q@h/").username);

// Non-ASCII is UTF-8 percent-encoded everywhere.
console.log("utf8    ", new URL("https://a.b/é😀?é#é").href);
// An existing escape is not double-encoded.
console.log("escaped ", new URL("https://a.b/a%20b").pathname, new URL("https://a.b/a%zz").pathname);

// Tabs and newlines are REMOVED from the input before parsing, not encoded.
console.log("strip   ", new URL("https://a.b/x\ty\nz\r!").pathname);

// The scheme and host are case-insensitive and reported lower-case, and a
// special scheme's default port is not part of the serialization.
console.log("case    ", new URL("HTTPS://ExAmPlE.COM/").href);
console.log("port    ", new URL("https://e.com:443/").host, new URL("http://e.com:80/").host, new URL("http://e.com:8080/").host);

// For a special scheme a backslash is a path separator — in the authority too,
// where it ends the userinfo — but NOT in the query or fragment.
console.log("slash   ", new URL("https://a.b/x\\y").pathname, new URL("https://a.b/?x\\y").search);

// Relative resolution and dot segments are unchanged by any of the above.
console.log("relative", new URL("../x", "https://e.com/a/b/c").href, new URL("/z", "https://e.com/a/b").href);
console.log("dots    ", new URL("https://e.com/a/./b/../c").pathname);
console.log("params  ", new URL("https://e.com/?a=1&b=2").searchParams.get("b"), new URLSearchParams("a=1&a=3").getAll("a").join(","));

// `btoa`/`atob` existed only as `require('buffer').btoa`; node exposes both as
// globals, so a bare `btoa('abc')` was a ReferenceError.
console.log("base64  ", btoa("abc"), atob("YWJj"), btoa(""), atob(btoa("Hello, World!")));
console.log("padding ", btoa("a"), btoa("ab"), btoa("abc"));
console.log("bytes   ", btoa("\x00\x01\xff"), Array.from(atob("AAH/")).map((c) => c.charCodeAt(0)).join(","));
console.log("same    ", btoa("abc") === require("buffer").btoa("abc"), typeof globalThis.atob);

// `typeof` on a builtin module namespace. A namespace is modelled as a callable
// unless it is on an explicit non-callable list, and every sub-path module plus
// a dozen later-added ones were missing from it — so `typeof require('tls')`
// answered "function". Measured by taking `typeof` of every builtin module.
const mods = ["path", "path/posix", "path/win32", "fs", "fs/promises", "os", "util",
  "stream/promises", "stream/consumers", "stream/web", "timers", "timers/promises",
  "dns", "dns/promises", "https", "http2", "tls", "dgram", "cluster",
  "worker_threads", "readline", "repl", "vm", "domain", "trace_events", "inspector"];
console.log("ns-object", mods.every((m) => typeof require(m) === "object"), mods.length);
// `assert` is genuinely callable — it was on that list and should not have been.
console.log("ns-assert", typeof require("assert"), typeof require("assert/strict"));
require("assert")(true);
console.log("ns-call  ", "assert(true) passed", typeof require("assert").strictEqual);
// The cross-links keep working and keep their own flavor.
console.log("ns-flavor", require("path").win32.sep, require("path").posix.sep, require("path").win32.join("a", "b"));

// `querystring`. Five behaviours were wrong, two of them security-relevant.
const qs = require("querystring");

// `parse` returns a NULL-prototype object, so a `__proto__` or `constructor`
// key in the input is an ordinary own property rather than a reference to
// something inherited. It was inheriting Object.prototype.
const hostile = qs.parse("__proto__=polluted&constructor=x");
console.log("qs-proto", Object.getPrototypeOf(qs.parse("a=1")) === null, Object.keys(hostile).join(","));
console.log("qs-safe ", hostile.__proto__, {}.polluted);

// `maxKeys` (default 1000, 0 means unlimited) caps how many distinct keys are
// kept. It was ignored, so a hostile query string could allocate without bound.
const many = Array.from({ length: 1200 }, (_, i) => "k" + i + "=1").join("&");
console.log("qs-maxk ", Object.keys(qs.parse(many)).length,
  Object.keys(qs.parse("a=1&b=2&c=3", "&", "=", { maxKeys: 2 })).length,
  Object.keys(qs.parse("a=1&b=2&c=3", "&", "=", { maxKeys: 0 })).length);

// Only a string, number, bigint or boolean serializes; null, undefined, an
// object and a symbol all become the EMPTY string. Running everything through
// String() emitted the text "null" and "[object Object]", both of which parse
// back as real data.
console.log("qs-types", qs.stringify({ s: "x", n: 1, b: true, g: 10n, nul: null, und: undefined, o: {}, y: Symbol("s") }));
// The same rule applies element-by-element inside an array value.
console.log("qs-array", qs.stringify({ a: [1, null, {}, "x"] }));

// `unescape` is percent-decoding only — a `+` stays a `+`. Only `parse` reads
// it as a space, that being a form-encoding rule about the pair syntax rather
// than about percent-escapes.
console.log("qs-plus ", qs.unescape("a+b"), qs.unescape("a%2Bb"), JSON.stringify(qs.parse("a+b=c+d")));
console.log("qs-round", JSON.stringify(qs.parse(qs.stringify({ a: "x y", b: "&=?" }))));

// `for-of` over a NATIVE-tagged iterable. These dispatch their methods through
// the stdlib table rather than a property map, and the loop probed
// `Symbol.iterator` with a stored-property lookup — so it found nothing and
// fell through to materializing the value, which THREW for `URLSearchParams`.
// Spreading the same object already worked, because that path resolves the
// property properly and this one did not.
const params = new URLSearchParams("a=1&b=2");
const walked = [];
for (const [k, v] of params) walked.push(k + "=" + v);
console.log("usp-forof", walked.join(","), [...params].length);
const headers = new Headers([["x-one", "1"], ["x-two", "2"]]);
const seenHeaders = [];
for (const [k, v] of headers) seenHeaders.push(k + ":" + v);
console.log("hdr-forof", seenHeaders.sort().join(","));
// The iterators these expose directly work the same way.
console.log("usp-views", [...params.keys()].join(","), [...params.values()].join(","), [...params.entries()].length);
// And the paths that were already correct stay correct: arrays and strings
// keep the direct route, and a generator is still its own iterator.
const mixed = [];
for (const c of "ab") mixed.push(c);
for (const b of Buffer.from("hi")) mixed.push(b);
for (const n of new Uint8Array([7])) mixed.push(n);
for (const g of (function* () { yield "g"; })()) mixed.push(g);
console.log("others  ", mixed.join(","));
