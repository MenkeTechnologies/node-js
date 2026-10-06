// WHATWG URL parsing (opaque paths, scheme-relative and same-scheme references,
// fragment-only references keep the base query, blob: origins), the component
// setters' state-override parses, url.format(URL, options).
const cases = [["//h/p","https://a"],["//h","http://x/y"],["x:",undefined],["x:y/z",undefined],["mailto:a@b",undefined],["/p?q","http://h/a/b"],["?q","http://h/a/b?z#f"],["#f","http://h/a?b"],["","http://h/a?b#c"],["..","http://h/a/b/c"],["http:foo","http://h/a/"],["http://[::1]:80/",undefined],["http://1.2.3.4.5/",undefined],["http://0x7f.1/",undefined],["HTTP://EXAMPLE.com:0080/%7e?%7e",undefined],["file:///a/b/../c",undefined],["blob:http://x/y",undefined],["data:text/plain,hi",undefined],["javascript:alert(1)",undefined],["ws://h:80/",undefined],["https://h:443",undefined],["http://a b/",undefined],["http://h/ä?ä#ä",undefined],["non-spec://h/a/../b",undefined],["//",undefined]];
for (const [i, b] of cases) { try { const u = new URL(i, b); console.log(JSON.stringify([i, u.href, u.origin, u.protocol, u.host, u.pathname, u.search, u.hash])) } catch (e) { console.log(i, e.code, e.message) } }
console.log(URL.canParse("x:"), URL.canParse("//h"), URL.canParse("//h", "http://a"), URL.parse("bad"), URL.parse("/p", "http://h").href);
const url = require("url"); const u = new URL("https://us:pw@h.com:8080/p?q=1#h");
console.log(url.format(u), url.format(u, {auth:false}), url.format(u, {search:false, fragment:false}), url.format(u, {unicode:true}), url.format({protocol:"http", host:"h", pathname:"/x", query:{a:1}}));
console.log(url.parse("http://u@h:1/p?a=1#x").query, url.parse("http://h/p?a=1", true).query, url.resolve("http://a/b/c", "../d"), url.fileURLToPath("file:///tmp/a%20b"), url.pathToFileURL("/tmp/a b").href);

const u3=new URL("https://us:pw@xn--r8jz45g.jp:8080/p?q=1#h");
console.log(url.format(u3,{unicode:true}), url.format(u3,{auth:0, search:null}), url.format(new URL("mailto:a@b?x#y"),{fragment:false}));
for (const s of ["http://h/a/./b/../c", "http://h/%2e%2E/x", "foo://h/a/../b", "foo:/a/../b", "http://h?#", "HTTP://H/", "http://h:65536/", "http://[::ffff:1.2.3.4]/", "http://ex%41mple.com/", "https://ä.com/ä", "sc://ñ/", "http://h/ /x", "http://h/\u0000", "http://h/a\\b", "ws:h", "file://localhost/x", "file:c:/x", "about:blank", "http://@h/", "http://:@h/", "http://u:@h/"]) {
  try { const x = new URL(s); console.log(JSON.stringify([s, x.href, x.origin, x.host, x.pathname, x.search, x.hash])) } catch (e) { console.log(JSON.stringify(s), e.message) }
}
const m = new URL("mailto:a@b"); m.pathname = "c@d"; m.search = "s"; m.hash = "h"; console.log(m.href);
const v = new URL("http://h/p"); v.protocol = "foo"; v.hostname = "z"; v.port = "99"; v.username = "u v"; console.log(v.href, v.origin);
const u4=new URL("http://h/p"); try { u4.href = "bad" } catch (e) { console.log(e.code, e.message) } console.log(u4.href); u4.search="?a b"; u4.hash="x y"; u4.pathname="q r"; console.log(u4.href); u4.port="abc"; u4.port="12x"; console.log(u4.port); u4.hostname="a b"; console.log(u4.hostname); u4.protocol="https"; console.log(u4.href); u4.host="n:1"; console.log(u4.href); u4.password="p@"; console.log(u4.href)
