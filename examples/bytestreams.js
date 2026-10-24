// Byte streams (type: 'bytes'): a BYOB reader, a byobRequest per pull sized by
// autoAllocateChunkSize, respond(), read({ min }), a Uint16Array view, the
// default reader of a byte stream, tee; and FileHandle.readableWebStream() as
// node's byte stream. byobRequest was always null and readableWebStream was a
// default stream.
const fsp = require('fs/promises');
const rs = new ReadableStream({
  type: 'bytes',
  autoAllocateChunkSize: 4,
  pull(c) {
    const req = c.byobRequest;
    console.log('req', req !== null, req && req.view.constructor.name, req && req.view.byteLength);
    req.view[0] = 7; req.view[1] = 8;
    req.respond(2);
    if (++this.n === 3) c.close();
  },
  n: 0,
});
(async () => {
  const reader = rs.getReader({ mode: 'byob' });
  console.log(reader.constructor.name);
  let r = await reader.read(new Uint8Array(10));
  console.log(r.done, r.value.constructor.name, [...r.value], r.value.buffer.byteLength);
  r = await reader.read(new Uint16Array(4));
  console.log(r.done, r.value.constructor.name, r.value.length);
  r = await reader.read(new Uint8Array(3), { min: 1 });
  console.log(r.done, [...r.value]);
  r = await reader.read(new Uint8Array(3));
  console.log(r.done, r.value && r.value.byteLength);
  const d = new ReadableStream({ type: 'bytes', start(c) { c.enqueue(new Uint8Array([1, 2, 3])); c.close(); } });
  const dr = d.getReader();
  console.log(JSON.stringify(await dr.read()), JSON.stringify(await dr.read()));
  try { new ReadableStream().getReader({ mode: 'byob' }); } catch (e) { console.log(e.code, e.message); }
  try { new ReadableStream({ type: 'bytes' }).getReader({ mode: 'x' }); } catch (e) { console.log(e.code, e.message); }
  try { await new ReadableStream({ type: 'bytes' }).getReader({ mode: 'byob' }).read(new Uint8Array(2), { min: 3 }); } catch (e) { console.log(e.name, e.code, e.message); }
  const t = new ReadableStream({ type: 'bytes', start(c) { c.enqueue(new Uint8Array([9])); c.close(); } });
  const [a, b] = t.tee();
  console.log([...(await a.getReader().read()).value], [...(await b.getReader().read()).value]);

  const fh = await fsp.open(__filename);
  const fileStream = fh.readableWebStream();
  const br = fileStream.getReader({ mode: 'byob' });
  console.log(br.constructor.name);
  const { value } = await br.read(new Uint8Array(5));
  console.log(value.constructor.name, value.byteLength, String.fromCharCode(...value));
  br.releaseLock();
  const fr = fileStream.getReader();
  const c = await fr.read();
  console.log(c.value.constructor.name, c.value.byteLength <= 16384, c.value.buffer.byteLength);
  await fh.close();
  console.log(await fr.read().then((x) => x.done, (e) => e.message));
})();
