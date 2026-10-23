// An AsyncLocalStorage store follows the async work started inside `run`
// (or after `enterWith`): timers, intervals, immediates, nextTick, promise
// jobs (the frame `.then`/`await` registered in, not the resolver's), I/O and
// fs/promises completions — node's AsyncContextFrame. Event listeners run in
// the emitter's context. `AsyncLocalStorage.snapshot()`/`bind()`,
// `AsyncResource` (`runInAsyncScope`, `bind`, `AsyncResource.bind`) capture
// the frame they are made in. node-js tracked the store for synchronous code
// only, and the two statics did not exist.
// The async lines are sorted: their interleaving is the event loop's, not the
// store's.
const { AsyncLocalStorage, AsyncResource } = require('async_hooks');
const als = new AsyncLocalStorage();
const out = [];
const log = (...a) => out.push(a.map((x) => JSON.stringify(x)).join(' '));
let later;
als.run({ id: 1 }, () => {
  log('sync', als.getStore());
  setTimeout(() => log('timeout', als.getStore()), 1);
  setImmediate(() => log('immediate', als.getStore()));
  process.nextTick(() => log('tick', als.getStore()));
  Promise.resolve().then(() => log('then', als.getStore()));
  (async () => { await null; log('await', als.getStore()); await new Promise((r) => setTimeout(r, 2)); log('await2', als.getStore()); })();
  queueMicrotask(() => log('micro', als.getStore()));
  require('fs').stat(__filename, () => log('fs cb', als.getStore()));
  require('fs/promises').stat(__filename).then(() => log('fsp', als.getStore()));
  const ee = new (require('events'))(); ee.on('x', () => log('emit', als.getStore())); later = ee;
  const iv = setInterval(() => { log('interval', als.getStore()); clearInterval(iv); }, 1);
  let resolveIt; const p = new Promise((r) => { resolveIt = r; });
  globalThis.resolveIt = resolveIt; p.then(() => log('then-registered-inside', als.getStore()));
});
als.run('other', () => { later.emit('x'); globalThis.resolveIt(); });
log('outside', als.getStore());
als.run(2, () => als.run(3, () => log('nested', als.getStore())));
log('exit', als.run(9, () => als.exit(() => als.getStore())));
const s2 = new AsyncLocalStorage();
s2.run('A', () => { s2.enterWith('E'); log('enterWith', s2.getStore()); setTimeout(() => log('enterWith later', s2.getStore()), 1); });
log('after enterWith run', s2.getStore());
const snap = als.run('S', () => AsyncLocalStorage.snapshot()); log('snapshot', snap(() => als.getStore()));
const bound = als.run('B', () => AsyncLocalStorage.bind(() => als.getStore())); log('bind', bound());
const res = als.run('R', () => new AsyncResource('X')); log('resource', res.runInAsyncScope(() => als.getStore()), res.bind(() => als.getStore())(), AsyncResource.bind(() => als.getStore())());
// A promise resolved from ANOTHER context resumes the `await` (and runs the
// `.then`) in the context that registered it.
let resolveOut; const pOut = new Promise((r) => { resolveOut = r; });
als.run('A', async () => { await pOut; log('await-resolved-elsewhere', als.getStore()); });
als.run('A2', () => { pOut.then(() => log('then-resolved-elsewhere', als.getStore())); });
als.run('B', () => resolveOut());
const a2 = new AsyncLocalStorage(); a2.run(1, () => { a2.disable(); log('disabled', a2.getStore()); });
setTimeout(() => { const i = out.indexOf('"disabled" '); console.log(out.slice(0, i + 1).join('\n')); console.log('--'); console.log(out.slice(i + 1).sort().join('\n')); }, 30);
