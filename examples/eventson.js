// `events.on(emitter, event[, options])`, a port of node's: the event's
// argument arrays as an async iterator, `break`/`return()` removing its
// listeners, an `error` rethrown once, `options.close`, `options.signal`
// (AbortError), the option checks, and EventTarget sources. It was absent.
const { on, EventEmitter } = require('events');
(async () => {
  const e = new EventEmitter();
  process.nextTick(() => { e.emit('x', 1, 2); e.emit('x', 3); e.emit('y'); e.emit('x', 4); });
  const got = [];
  for await (const args of on(e, 'x')) { got.push(args); if (got.length === 3) break; }
  console.log(JSON.stringify(got), e.listenerCount('x'), e.listenerCount('error'));
  const e2 = new EventEmitter();
  const it = on(e2, 'x');
  process.nextTick(() => { e2.emit('x', 'a'); e2.emit('error', new Error('boom')); });
  try { for await (const a of it) console.log('val', a); } catch (err) { console.log('err', err.message); }
  console.log(await it.next());
  const e3 = new EventEmitter();
  const it3 = on(e3, 'x', { close: ['end'] });
  process.nextTick(() => { e3.emit('x', 1); e3.emit('end'); e3.emit('x', 2); });
  const g3 = []; for await (const a of it3) g3.push(a); console.log(JSON.stringify(g3), e3.listenerCount('end'));
  const ac = new AbortController();
  const it4 = on(new EventEmitter(), 'x', { signal: ac.signal });
  setTimeout(() => ac.abort('r'), 1);
  try { await it4.next(); } catch (err) { console.log(err.name, err.code, err.cause); }
  for (const bad of [5, { signal: 5 }, { highWaterMark: 0 }]) { try { on(new EventEmitter(), 'x', bad); } catch (err) { console.log(err.code, err.message); } }
  ac.abort(); try { on(new EventEmitter(), 'x', { signal: ac.signal }); } catch (err) { console.log(err.name); }
  const et = new EventTarget(); const it5 = on(et, 'foo');
  process.nextTick(() => et.dispatchEvent(new Event('foo')));
  const r5 = await it5.next(); console.log(r5.value[0].type, r5.done); await it5.return();
  console.log(typeof on, on.length, Symbol.asyncIterator in it5);
  try { it5.throw(5); } catch (err) { console.log(err.code, err.message); }
})();
