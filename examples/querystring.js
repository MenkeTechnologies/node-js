// querystring is node's single-pass scanner: multi-unit separators, a
// `maxKeys` budget counting PAIRS, decoding only a part that held an escape,
// a custom decodeURIComponent/encodeURIComponent, falsy separators meaning the
// default, and `encode`/`decode` being the same functions as
// `stringify`/`parse`. Each line printed something else before the port.
const qs = require("querystring");
const t = (label, f) => {
  try {
    console.log(label, JSON.stringify(f()));
  } catch (e) {
    console.log(label, e.name, e.message);
  }
};
const many = Array.from({ length: 1500 }, (_, i) => `k${i}=1`).join("&");
t("defaultMax", () => [Object.keys(qs.parse(many)).length, Object.keys(qs.parse(many, ";")).length]);
t("nullSep   ", () => qs.parse("a=1&b=2&c=3", null, null, { maxKeys: 2 }));
t("falsySep  ", () => qs.parse("a=1&b=2", 0, false));
t("pairBudget", () => qs.parse("a=1&&b=2&c", "&", "=", { maxKeys: 2 }));
t("dupBudget ", () => qs.parse("a=1&a=2&b=3", null, null, { maxKeys: 2 }));
t("maxStr    ", () => qs.parse("a=1&b=2", "&", "=", { maxKeys: "1" }));
t("maxFrac   ", () => Object.keys(qs.parse(many, "&", "=", { maxKeys: 1.5 })).length);
t("notString ", () => qs.parse(5));
t("multiSep  ", () => qs.parse("a=1&;b=2&;c", "&;"));
t("multiEq   ", () => qs.parse("a==>1&b=>2=>3", "&", "=>"));
for (const s of ["&&=&a&=b&a=", "a=b=c", "a%2=1", "a+%20b=c+%2B", "a=%zz&b=%E0%A4&c=%41%", "__proto__=1&constructor=2"]) {
  t(`parse ${JSON.stringify(s)}`, () => qs.parse(s));
}
t("customDec ", () => qs.parse("a=%41&b=x+y", null, null, { decodeURIComponent: (s) => `[${s}]` }));
t("decThrows ", () => qs.parse("a=%41", null, null, { decodeURIComponent: () => { throw new Error("x"); } }));
t("decNumber ", () => qs.parse("a=1&a=2", null, null, { decodeURIComponent: () => 7 }));
t("afterThrow", () => qs.escape(Symbol("s")));
t("stringify ", () => qs.stringify({ b: 2, a: [1, , 3], 1: "one", c: [], d: "é ", e: 1e21, f: -0, g: [null, {}], h: NaN, i: 1n }));
t("strSep    ", () => [qs.stringify({ a: 1, b: 2 }, ";", ":"), qs.stringify({ a: 1, b: 2 }, null, null)]);
t("customEnc ", () => qs.stringify({ a: "x y", b: null }, null, null, { encodeURIComponent: (s) => `<${s.toUpperCase()}>` }));
t("getter    ", () => qs.stringify({ get x() { return "g"; } }));
t("nonObject ", () => [qs.stringify("x"), qs.stringify(null), qs.stringify(() => 1), qs.stringify(["a", "b"])]);
t("escape    ", () => [qs.escape(12), qs.escape(null), qs.escape({ toString() { return "a b"; } })]);
t("unescape  ", () => [qs.unescape("a+b"), qs.unescape("%zz%41"), qs.unescapeBuffer("a+b", 1).toString()]);
t("aliases   ", () => [qs.encode === qs.stringify, qs.decode === qs.parse, qs.encode.name, qs.decode.name]);
