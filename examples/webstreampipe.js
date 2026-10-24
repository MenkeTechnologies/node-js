// pipeTo/pipeThrough with preventClose and an AbortSignal, a writable's
// backpressure (desiredSize, ready), a source error aborting the destination,
// ReadableStream.from over sync and async iterables, values({ preventCancel })
// on %AsyncIteratorPrototype%, require('stream/web').ReadableStreamTee with
// cloneForBranch2, and the constructors refusing a call without new. pipeTo
// and tee drained their source synchronously and ReadableStreamTee was absent.
const util = require('util');
(async () => {
  const written = [];
  const ws = new WritableStream({
    async write(chunk) { await new Promise((r) => setTimeout(r, 1)); written.push(chunk); },
    close() { written.push('closed'); },
  }, { highWaterMark: 2 });
  const w = ws.getWriter();
  console.log('ds', w.desiredSize);
  w.write('a'); console.log('ds', w.desiredSize);
  w.write('b'); console.log('ds', w.desiredSize);
  await w.ready; console.log('ready', w.desiredSize);
  await w.close(); console.log(written);
  console.log(util.inspect(ws));
  const src = () => new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close(); } });
  const out = [];
  const dst = new WritableStream({ write(c) { out.push(c); }, close() { out.push('end'); } });
  await src().pipeTo(dst, { preventClose: true });
  console.log(out, dst.locked);
  const ac = new AbortController();
  const slow = new ReadableStream({ pull(c) { return new Promise((r) => setTimeout(() => { try { c.enqueue('x'); } catch {} r(); }, 5)); } });
  const sink = [];
  const p = slow.pipeTo(new WritableStream({ write(c) { sink.push(c); } }), { signal: ac.signal });
  setTimeout(() => ac.abort(), 40);
  try { await p; } catch (e) { console.log('aborted', e.name, sink.length > 0); }
  const errSrc = new ReadableStream({ start(c) { c.error(new Error('bad')); } });
  const aborts = [];
  await errSrc.pipeTo(new WritableStream({ abort(r) { aborts.push(r.message); } })).catch((e) => console.log('rej', e.message));
  console.log(aborts);
  const ts = new TransformStream({ transform(ch, c) { c.enqueue(ch * 10); }, flush(c) { c.enqueue(-1); } });
  const res = [];
  for await (const v of src().pipeThrough(ts)) res.push(v);
  console.log(res);
  try { src().pipeThrough({}); } catch (e) { console.log(e.code, e.message); }
  try { await src().pipeTo({}); } catch (e) { console.log(e.code, e.message); }
  const it = src().values({ preventCancel: true });
  console.log(Object.getPrototypeOf(Object.getPrototypeOf(it)) === Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype));
  console.log(await it.next(), await it.return('r'));
  const got = []; for await (const v of ReadableStream.from([1, 2, 3])) got.push(v); console.log(got);
  for await (const v of ReadableStream.from((async function* () { yield 'q'; })())) console.log('from-async', v);
  try { ReadableStream.from(5); } catch (e) { console.log(e.code, e.message); }
  const web = require('stream/web');
  console.log(Object.keys(web).length, typeof web.ReadableStreamTee);
  const [t1, t2] = web.ReadableStreamTee(new ReadableStream({ start(c) { c.enqueue({ k: 1 }); c.close(); } }), true);
  const v1 = (await t1.getReader().read()).value, v2 = (await t2.getReader().read()).value;
  console.log(v1 === v2, JSON.stringify(v2));
  try { web.ReadableStreamTee(1); } catch (e) { console.log(e.code, e.message); }
  for (const f of [() => ReadableStream(), () => web.WritableStream(), () => new ReadableStreamBYOBRequest()]) {
    try { f(); } catch (e) { console.log(e.name, e.code, e.message); }
  }
  // A writer's pending `ready` and `closed` reject with the abort reason
  // (node checks them with `isPromisePending`, V8's promise state).
  const hw = new WritableStream({}, { highWaterMark: 0 }).getWriter();
  const heldReady = hw.ready.then(() => 'ready', (e) => 'ready rejected ' + e);
  const heldClosed = hw.closed.then(() => 'closed', (e) => 'closed rejected ' + e);
  await hw.abort('why');
  console.log(await heldReady, await heldClosed);
  // util.styleText accepts a web stream as its `stream` option (node's
  // isReadableStream || isWritableStream || isNodeStream).
  for (const s of [new WritableStream(), new ReadableStream(), {}]) {
    try { console.log(JSON.stringify(util.styleText('red', 'x', { stream: s }))); } catch (e) { console.log(e.code, e.message); }
  }
})();
