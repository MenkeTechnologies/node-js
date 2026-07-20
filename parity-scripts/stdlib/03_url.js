// url: WHATWG URL class components and searchParams.
const { URL } = require("url");

const u = new URL("https://user:pass@example.com:8080/p/a/t/h?q=1&r=2#frag");
console.log(u.protocol);
console.log(u.username, u.password);
console.log(u.hostname, u.port);
console.log(u.pathname);
console.log(u.search);
console.log(u.hash);
console.log(u.href);
console.log(u.origin);

const sp = u.searchParams;
console.log(sp.get("q"), sp.get("r"));
console.log(sp.has("q"), sp.has("z"));
sp.append("s", "3");
console.log(sp.toString());
console.log([...sp.keys()].join(","));

const rel = new URL("../c", "https://example.com/a/b/d");
console.log(rel.href);
