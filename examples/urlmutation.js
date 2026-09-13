// A `URL` is LIVE: assigning a component rewrites the derived fields, and its
// `searchParams` writes back. Components were plain data properties with no
// write interception and `searchParams` was a snapshot taken at construction,
// so `u.pathname = '/p'` read back as `/p` while `u.href` still showed the old
// path, and `u.searchParams.set(...)` went nowhere.
const u = new URL("https://h.com/");
u.pathname = "/p";
console.log("pathname", u.pathname, u.href);
u.port = "99";
u.search = "?a=1";
u.hash = "#z";
console.log("all     ", u.href, u.host, u.origin);
// `protocol`, `search` and `hash` are normalized on assignment: the delimiter
// is supplied when it was left off, and the empty string clears the component.
const n = new URL("https://h.com/p?a=1#f");
n.protocol = "http";
n.hash = "z";
console.log("delims  ", n.protocol, n.hash, n.href);
n.search = "";
console.log("cleared ", JSON.stringify(n.search), n.href, n.searchParams.size);

// One `URLSearchParams` per URL for the life of the URL — the same object
// before and after the query is replaced through `url.search`.
const v = new URL("https://h.com/?a=1");
const p = v.searchParams;
v.search = "?x=9";
console.log("identity", p === v.searchParams, p.get("x"), p.size);
// …and every mutation of it rewrites the owner's `search` and `href`.
p.append("y", "2");
console.log("append  ", v.href);
p.delete("x");
console.log("delete  ", v.href, v.search, p.size);
p.set("y", "3");
console.log("set     ", v.href, [...p].map((e) => e.join("=")).join("|"));
// A standalone `URLSearchParams` has no owner to notify.
const w = new URLSearchParams("z=1");
w.set("y", "2");
console.log("detached", w.toString(), w.size);

// `URL.canParse`/`URL.parse` are the non-throwing form of the constructor.
// Neither existed: `URL.canParse` was a TypeError rather than a boolean.
console.log("statics ", typeof URL.canParse, typeof URL.parse);
console.log("canParse", URL.canParse("https://a.com"), URL.canParse("nope"),
  URL.canParse("/p", "https://a.com"));
console.log("parse   ", String(URL.parse("https://a.com/x")), URL.parse("nope"),
  String(URL.parse("/p", "https://a.com")));
