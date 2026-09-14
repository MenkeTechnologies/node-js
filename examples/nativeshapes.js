// The SHAPE of a native instance, as distinct from what its methods answer.
//
// Every class here built a plain object hanging off `Object.prototype` with its
// public fields as own enumerable data properties. Three things followed, all
// of them wrong and none of them visible from calling the methods:
// `x.constructor.name` read `"Object"`, a chain walk found none of the class's
// methods, and every field leaked into `Object.keys`, into a spread, and into
// the JSON of anything that merely HELD one.
const c = require("crypto");
const vm = require("vm");

const t = (l, f) => {
  try {
    console.log(l, JSON.stringify(f()));
  } catch (e) {
    console.log(l, "THROW", e.constructor.name, JSON.stringify([e.code, e.message]));
  }
};
const shape = (o) => [o.constructor.name, o[Symbol.toStringTag] || null, Object.keys(o), JSON.stringify(o)];
// Node's own classes keep INTERNAL own properties (`_options`, `_events`) that
// this runtime has no equivalent for and should not invent, so for those the
// check is the class identity rather than the own-key list.
const named = (o) => [o.constructor.name, o[Symbol.toStringTag] || null];

// ── the instance is linked to its class prototype ────────────────────────────
t("hash      ", () => named(c.createHash("md5")));
t("hmac      ", () => named(c.createHmac("md5", "k")));
t("cipher    ", () => named(c.createCipheriv("aes-256-cbc", Buffer.alloc(32), Buffer.alloc(16))));
t("sign      ", () => named(c.createSign("sha256")));
t("script    ", () => named(new vm.Script("1")));
t("params    ", () => shape(new URLSearchParams("a=1")));
// A `StringDecoder` really does keep `encoding` as an own property.
t("decoder-own", () => [Object.keys(new (require("string_decoder").StringDecoder)("utf8"))]);
t("decoder   ", () => named(new (require("string_decoder").StringDecoder)("utf8")));
t("linked    ", () => {
  const h = c.createHash("md5");
  return [Object.getPrototypeOf(h) === c.Hash.prototype, h instanceof c.Hash, Object.getPrototypeOf(h) === Object.prototype];
});

// ── accessors live on the prototype, not on the instance ─────────────────────
t("controller", () => shape(new AbortController()));
t("signal    ", () => shape(new AbortController().signal));
t("held      ", () => JSON.stringify({ ctrl: new AbortController() }));
t("ac-desc   ", () => {
  const d = Object.getOwnPropertyDescriptor(AbortController.prototype, "signal");
  return [!!d.get, !!d.set, d.enumerable, d.configurable];
});
// `onabort` is the one of the three that is WRITABLE.
t("onabort   ", () => {
  const a = new AbortController();
  let hit = 0;
  a.signal.onabort = () => hit++;
  a.abort("why");
  return [hit, a.signal.aborted, a.signal.reason, Object.keys(a.signal)];
});
t("abort-dflt", () => {
  const a = new AbortController();
  a.abort();
  return [a.signal.aborted, a.signal.reason.name];
});

// ── URL: twelve accessors, two of them read-only ─────────────────────────────
const u = new URL("https://user:pw@ex.com:8080/a/b?x=1#f");
t("url-shape ", () => shape(u));
t("url-read  ", () => [u.protocol, u.username, u.password, u.host, u.hostname, u.port, u.pathname, u.search, u.hash, u.origin]);
// A prototype member is ENUMERABLE, so `for-in` over a URL walks its accessors
// and its two methods. Hiding them made the loop find nothing at all.
t("url-forin ", () => { const ks = []; for (const k in u) ks.push(k); return ks; });
t("url-desc  ", () => {
  const d = (k) => {
    const x = Object.getOwnPropertyDescriptor(URL.prototype, k);
    return [!!x.get, !!x.set];
  };
  return [d("href"), d("origin"), d("pathname"), d("searchParams")];
});
// Assigning a component rewrites the fields DERIVED from it.
t("set-path  ", () => { const v = new URL("http://a/p?x=1"); v.pathname = "/z"; return [v.href, v.pathname]; });
t("set-proto ", () => { const v = new URL("http://a/p"); v.protocol = "https"; return v.href; });
t("set-port  ", () => { const v = new URL("http://a/p"); v.port = "70"; return [v.href, v.host]; });
// `host` is TWO fields, and `href` is the whole URL: assigning either has to do
// more than store a string, or the object contradicts itself.
t("set-host  ", () => { const v = new URL("http://a/p"); v.host = "b:99"; return [v.href, v.hostname, v.port]; });
t("set-href  ", () => {
  const v = new URL("http://a/p");
  v.href = "https://b:1/q?r=2#s";
  return [v.protocol, v.host, v.port, v.pathname, v.search, v.hash, v.origin, v.searchParams.get("r")];
});
t("set-search", () => { const v = new URL("http://a/p"); v.search = "q=2"; return [v.href, v.searchParams.get("q")]; });
t("params-mut", () => { const v = new URL("http://a/p?x=1"); v.searchParams.set("y", "2"); return [v.href, v.search]; });
// The two read-only ones ignore an assignment in sloppy mode.
t("url-ro    ", () => {
  const v = new URL("http://a/p");
  const sp = v.searchParams;
  v.origin = "zzz";
  v.searchParams = "zzz";
  return [v.origin, v.searchParams === sp];
});

// ── TextDecoder: a label is not an encoding, and the options were ignored ─────
t("td-default", () => { const d = new TextDecoder(); return [d.encoding, d.fatal, d.ignoreBOM, Object.keys(d)]; });
t("td-opts   ", () => { const d = new TextDecoder("utf-8", { fatal: 1, ignoreBOM: "y" }); return [d.fatal, d.ignoreBOM]; });
t("td-labels ", () => ["utf8", "latin1", "iso-8859-1", "ascii", "ucs-2", "UTF-8"].map((l) => new TextDecoder(l).encoding));
t("td-unknown", () => new TextDecoder("nope-xyz"));
// windows-1252 is NOT latin1: 0x80..0x9F carry typography, not control codes.
t("td-cp1252 ", () => new TextDecoder("latin1").decode(new Uint8Array([0x80, 0x93, 0x99, 0xe9])));
t("td-fatal  ", () => new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array([0xff, 0xfe, 0xfd])));
t("td-lax    ", () => new TextDecoder().decode(new Uint8Array([0xff, 0xfe, 0xfd])));
t("td-bom    ", () => {
  const bytes = new Uint8Array([0xef, 0xbb, 0xbf, 97]);
  const kept = new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
  return [new TextDecoder().decode(bytes), kept.length, kept.charCodeAt(0)];
});
t("td-utf16  ", () => new TextDecoder("utf-16le").decode(new Uint8Array([0x68, 0, 0x69, 0])));
t("te        ", () => { const e = new TextEncoder(); return [e.encoding, Object.keys(e), Array.from(e.encode("aé"))]; });

// ── KeyObject: the class handed back is the LEAF, not the base ───────────────
const key = c.createSecretKey(Buffer.from("ab"));
t("key-shape ", () => shape(key));
t("key-size  ", () => [key.type, key.symmetricKeySize, key.asymmetricKeyType]);
// Comparing the PEM compared two empty strings for secret keys, so every pair
// of them was equal — including keys built from different bytes.
t("key-equals", () => [
  key.equals(c.createSecretKey(Buffer.from("ab"))),
  key.equals(c.createSecretKey(Buffer.from("ac"))),
  key.equals(c.createSecretKey(Buffer.from("abc"))),
]);
t("key-chain ", () => {
  const proto = Object.getPrototypeOf(key);
  return [proto.constructor.name, Object.getPrototypeOf(proto).constructor.name];
});
