// ReadableStream's underlying source is node's: start/pull/cancel are awaited,
// pull runs again only while desiredSize is positive (backpressure through
// the strategy's highWaterMark), and the queuing strategies' size is a getter.
// The previous implementation ran pull synchronously and never awaited it.
const log = [];
const rs = new ReadableStream({
  start(c) { log.push(['start', c.desiredSize]); },
  async pull(c) {
    log.push(['pull', c.desiredSize]);
    await new Promise((r) => setTimeout(r, 2));
    c.enqueue(log.length);
    if (log.length > 6) c.close();
  },
  cancel(r) { log.push(['cancel', r]); },
}, new CountQueuingStrategy({ highWaterMark: 2 }));
(async () => {
  const reader = rs.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    log.push(['read', done, value]);
    if (done) break;
  }
  console.log(JSON.stringify(log));
  console.log(String(rs), require('util').inspect(rs));
  const s = new CountQueuingStrategy({ highWaterMark: 3 });
  const d = Object.getOwnPropertyDescriptor(CountQueuingStrategy.prototype, 'size');
  console.log(typeof d.get, d.enumerable, s.size.name, s.size(), s.size === s.size, s.highWaterMark);
  const b = new ByteLengthQueuingStrategy({ highWaterMark: 16 });
  console.log(b.size(new Uint8Array(5)), b.size.name, require('util').inspect(b));
  try { new CountQueuingStrategy(); } catch (e) { console.log(e.code, e.message); }
  try { new CountQueuingStrategy({}); } catch (e) { console.log(e.code, e.message); }
  try { new ReadableStream({ type: 'nope' }); } catch (e) { console.log(e.code, e.message); }
  try { new ReadableStream({}, { highWaterMark: -1 }); } catch (e) { console.log(e.name, e.code, e.message); }
  try { ReadableStream.prototype.getReader.call({}); } catch (e) { console.log(e.code, e.message); }
  try { new ReadableStreamDefaultReader({}); } catch (e) { console.log(e.code, e.message); }
  try { new ReadableStreamDefaultController(); } catch (e) { console.log(e.code, e.message); }
  const r2 = new ReadableStream(); r2.getReader();
  try { r2.getReader(); } catch (e) { console.log(e.name, e.code, e.message); }
})();
