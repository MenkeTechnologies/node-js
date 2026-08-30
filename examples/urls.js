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
