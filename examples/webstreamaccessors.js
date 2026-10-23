// The web streams keep their state behind PROTOTYPE getters, as node's do:
// `locked` on ReadableStream/WritableStream, `readable`/`writable` on
// TransformStream and the codec streams (plus `encoding`/`fatal`/`ignoreBOM`
// on the text ones). They were own data properties of every instance, so
// `Object.keys(stream)` listed them, the prototypes did not, and the getter
// could not brand-check its receiver.
const sorted = (C) => Object.getOwnPropertyNames(C.prototype).sort().join();
for (const C of [ReadableStream, WritableStream, TransformStream, TextEncoderStream, TextDecoderStream, CompressionStream, DecompressionStream]) console.log(C.name, sorted(C));
const d = Object.getOwnPropertyDescriptor(ReadableStream.prototype, 'locked');
console.log(typeof d.get, d.set, d.enumerable, d.configurable);
const rs = new ReadableStream({ start(c) { c.enqueue('a'); c.close(); } });
console.log(Object.keys(rs), Object.hasOwn(rs, 'locked'), rs.locked, 'locked' in rs);
const r = rs.getReader(); console.log(rs.locked); r.releaseLock(); console.log(rs.locked);
try { d.get.call({}); } catch (e) { console.log(e.name, e.code, e.message); }
const ws = new WritableStream(); console.log(Object.keys(ws), ws.locked); ws.getWriter(); console.log(ws.locked);
const ts = new TransformStream(); console.log(Object.keys(ts), ts.readable instanceof ReadableStream, ts.writable instanceof WritableStream);
const te = new TextEncoderStream(); console.log(Object.keys(te), te.encoding, te.readable instanceof ReadableStream);
const td = new TextDecoderStream('utf-8', { fatal: false }); console.log(Object.keys(td), td.encoding, td.fatal, td.ignoreBOM);
(async () => {
  const out = rs.locked ? null : new ReadableStream({ start(c) { c.enqueue('xy'); c.close(); } }).pipeThrough(new TextEncoderStream());
  const chunks = []; for await (const c of out) chunks.push(...c); console.log(chunks);
  const cs = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode('hello')); c.close(); } }).pipeThrough(new CompressionStream('gzip')).pipeThrough(new DecompressionStream('gzip')).pipeThrough(new TextDecoderStream());
  let s = ''; for await (const c of cs) s += c; console.log(s);
})();
