// Every built-in iterator carries its prototype's brand: Object.prototype.toString,
// a Symbol.toStringTag read and util.inspect all name it, and a URLSearchParams
// iterator inspects as what it has left.
const util = require("util");
const show = (k, x) =>
  console.log(k, Object.prototype.toString.call(x), String(x[Symbol.toStringTag]), util.inspect(x));
const fd = new FormData();
fd.append("a", "1");
show("string", "ab"[Symbol.iterator]());
show("string generic", String.prototype[Symbol.iterator].call(12));
show("matchAll", "ab".matchAll(/a/g));
show("buffer", Buffer.from("a").entries());
show("typed", new Uint8Array(2).keys());
show("array", [1].values());
show("headers", new Headers({ x: "1" }).entries());
show("formdata", fd.entries());
show("mime params", new util.MIMEType("text/html;a=b").params.keys());
const usp = new URLSearchParams("a=1&b=2");
const e = usp.entries();
show("usp entries", e);
e.next();
show("usp after one", e);
e.next();
show("usp exhausted", e);
show("usp keys", usp.keys());
console.log([usp.values()], { x: usp.keys() });
const s = "ab"[Symbol.iterator]();
s.next();
console.log(util.inspect(s), [...s]);
