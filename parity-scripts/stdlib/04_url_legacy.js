// url: legacy url.parse / url.format / url.resolve.
const url = require("url");

const parsed = url.parse("https://example.com:8080/p/a/t/h?q=1&r=2#frag", true);
console.log(parsed.protocol);
console.log(parsed.host);
console.log(parsed.hostname);
console.log(parsed.port);
console.log(parsed.pathname);
console.log(parsed.search);
console.log(parsed.hash);
console.log(JSON.stringify(parsed.query));

const formatted = url.format({
  protocol: "https",
  hostname: "example.com",
  pathname: "/some/path",
  query: { a: "1", b: "2" },
});
console.log(formatted);

console.log(url.resolve("/one/two/three", "four"));
console.log(url.resolve("http://example.com/", "/one"));
