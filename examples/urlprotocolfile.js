// `url.protocol = 'file'` on a special URL (http, https, ws, wss, ftp) changes
// the scheme unless the URL has credentials or a port, as the WHATWG setter
// specifies; a non-special URL, an invalid scheme, and a change away from
// file stay refused. The url crate refused every change to file, so the
// assignment was silently ignored.
for (const s of ['http://a/', 'https://A.b/%7e', 'ws://x/y', 'ftp://h/p?x#f', 'http://u:p@a/', 'http://a:81/', 'file://h/p', 'foo://h/p']) {
  for (const p of ['file', 'FILE:junk', 'file x', 'file+x', '1file', 'fi le']) {
    const u = new URL(s);
    u.protocol = p;
    console.log(s, JSON.stringify(p), '->', u.href, u.protocol, u.host, u.origin);
  }
}
const w = new URL('http://a/b?c=1');
w.protocol = 'file';
console.log(w.pathname, w.search, w.searchParams.get('c'), String(w), JSON.stringify(w));
