// `options.signal` on the fs/promises file operations and FileHandle
// methods (validated; an aborted one rejects with node's AbortError),
// `fh.getAsyncId()`, and `fh.readableWebStream()` with its lock, closed-handle
// refusals and `autoClose`. All three were absent: the signal was ignored and
// the two methods did not exist.
const fsp = require('fs/promises');
(async () => {
  const ac = new AbortController(); ac.abort('why');
  for (const f of [() => fsp.readFile(__filename, { signal: ac.signal }), () => fsp.writeFile(require('path').join(require('os').tmpdir(), 'njfh-never-written'), 'x', { signal: ac.signal })]) {
    try { await f(); console.log('ok'); } catch (e) { console.log(e.name, e.code, e.message, e.cause, Object.keys(e), e instanceof Error, e.constructor.name, Object.getPrototypeOf(e) === Error.prototype); }
  }
  const fh = await fsp.open(__filename);
  for (const f of [() => fh.readFile({ signal: ac.signal }), () => fh.stat({ signal: ac.signal })]) {
    try { await f(); console.log('ok'); } catch (e) { console.log(e.name, e.code, e.message, e.cause); }
  }
  try { await fh.readFile({ signal: 5 }); } catch (e) { console.log(e.code, e.message); }
  console.log(typeof fh.getAsyncId(), fh.getAsyncId() === fh.getAsyncId());
  const rs = fh.readableWebStream();
  const chunks = []; for await (const c of rs) chunks.push(c.constructor.name + ':' + c.length);
  console.log(chunks.length > 0, chunks[0].split(':')[0], fh.fd !== -1);
  try { fh.readableWebStream(); } catch (e) { console.log(e.name, e.code, e.message); }
  await fh.close();
  try { fh.readableWebStream(); } catch (e) { console.log(e.name, e.code, e.message); }
  const fh2 = await fsp.open(__filename);
  const r2 = fh2.readableWebStream({ autoClose: true }); for await (const c of r2) {} 
  await new Promise((r) => setTimeout(r, 10)); console.log('autoClose', fh2.fd);
  try { (await fsp.open(__filename)).readableWebStream({ autoClose: 1 }); } catch (e) { console.log(e.code, e.message); }
})();
