// The modelled web/node class prototypes: members in node's order (measured:
// CLASS_PROTO_ORDER) — the stream readers', writers', controllers' and
// strategies' state as prototype GETTERS, not own properties — accessor
// getters named `get <member>`, and method `length`s (measured:
// CLASS_PROTO_ARITY). Methods listed before accessors, getters were named
// `@get@locked`, and every one of these methods' `length` was undefined.
// (Only members node-js implements are compared.)
const order = (n) => Object.getOwnPropertyNames(globalThis[n].prototype).join();
for (const n of ['ReadableStream', 'WritableStream', 'TransformStream', 'TextEncoderStream', 'TextDecoderStream', 'CompressionStream', 'Crypto', 'CryptoKey', 'EventTarget', 'Event', 'CustomEvent', 'AbortController', 'AbortSignal', 'ReadableStreamDefaultReader', 'WritableStreamDefaultWriter', 'CountQueuingStrategy']) console.log(n, order(n));
const fn = (n, k) => { const d = Object.getOwnPropertyDescriptor(globalThis[n].prototype, k); const f = d.value || d.get; return `${n}.${k}=${f.name}/${f.length}`; };
console.log([['ReadableStream', 'getReader'], ['ReadableStream', 'pipeTo'], ['ReadableStream', 'locked'], ['WritableStream', 'getWriter'], ['WritableStreamDefaultWriter', 'write'], ['WritableStreamDefaultWriter', 'ready'], ['Performance', 'mark'], ['Performance', 'timerify'], ['Performance', 'timeOrigin'], ['Performance', 'setResourceTimingBufferSize'], ['Crypto', 'getRandomValues'], ['Crypto', 'subtle'], ['SubtleCrypto', 'digest'], ['EventTarget', 'addEventListener'], ['AbortSignal', 'aborted'], ['Event', 'composedPath']].map(([n, k]) => fn(n, k)).join('\n'));
console.log(performance.clearResourceTimings(), performance.setResourceTimingBufferSize(10), typeof performance.timeOrigin, typeof require('perf_hooks').clearResourceTimings);
(async () => {
  const rs = new ReadableStream({ start(c) { console.log('ds', c.desiredSize, Object.keys(c)); c.enqueue('a'); c.close(); } });
  const r = rs.getReader(); console.log(Object.keys(r), r.closed instanceof Promise);
  console.log(await r.read()); await r.closed; console.log('closed ok');
  const ws = new WritableStream({ write() {} }); const w = ws.getWriter();
  console.log(Object.keys(w), typeof w.desiredSize, w.ready instanceof Promise, w.closed instanceof Promise);
  await w.write('x'); await w.close(); await w.closed; console.log('w closed');
  const s1 = new CountQueuingStrategy({ highWaterMark: 3 }), s2 = new ByteLengthQueuingStrategy({ highWaterMark: 5 });
  console.log(Object.keys(s1), s1.highWaterMark, s2.highWaterMark, s2.size(new Uint8Array(4)));
  const ts = new TransformStream({ start(c) { console.log('tds', typeof c.desiredSize, Object.keys(c)); } });
})();
