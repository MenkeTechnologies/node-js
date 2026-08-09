// [[AsyncGeneratorQueue]] ordering: .next/.return/.throw all enqueue, and the
// queue is drained in call order (ECMA-262 27.6.3.6 AsyncGeneratorEnqueue).
const log = (...a) => console.log(...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  // 1) next() then return() issued back-to-back, no await between.
  {
    async function* g() {
      try {
        yield 1;
        await sleep(5);
        yield 2;
        yield 3;
      } finally {
        log('  t1 finally');
      }
    }
    const it = g();
    const p1 = it.next();
    const p2 = it.return('R');
    const p3 = it.next();
    p1.then((v) => log('  t1 p1 settled', JSON.stringify(v)));
    p2.then((v) => log('  t1 p2 settled', JSON.stringify(v)));
    p3.then((v) => log('  t1 p3 settled', JSON.stringify(v)));
    log('t1', JSON.stringify(await Promise.all([p1, p2, p3])));
  }

  // 2) return() issued while an awaiting step is still in flight.
  {
    async function* g() {
      yield 1;
      await sleep(20);
      yield 2;
    }
    const it = g();
    const p1 = it.next();
    const p2 = it.next();      // this one suspends on the 20ms await
    const p3 = it.return('R'); // must wait for p2, not jump the queue
    log('t2', JSON.stringify(await Promise.all([p1, p2, p3])));
  }

  // 3) throw() interleaved with next(), caught inside the body.
  {
    async function* g() {
      try {
        yield 1;
        await sleep(5);
        yield 2;
      } catch (e) {
        log('  t3 caught', e);
        yield 'recovered';
      }
    }
    const it = g();
    const p1 = it.next();
    const p2 = it.throw('BOOM');
    const p3 = it.next();
    log('t3', JSON.stringify(await Promise.all([p1, p2, p3])));
  }

  // 4) throw() that is NOT caught: rejects, and later requests see done.
  {
    async function* g() {
      yield 1;
      await sleep(5);
      yield 2;
    }
    const it = g();
    const p1 = it.next();
    const p2 = it.throw(new Error('unhandled'));
    const p3 = it.next();
    const settled = await Promise.allSettled([p1, p2, p3]);
    log('t4', JSON.stringify(settled.map((s) => (s.status === 'fulfilled'
      ? { ok: s.value }
      : { err: String(s.reason) }))));
  }

  // 5) three next() then a return(), all issued synchronously.
  {
    async function* g() {
      for (let i = 0; i < 5; i++) { await sleep(1); yield i; }
    }
    const it = g();
    const ps = [it.next(), it.next(), it.next(), it.return('END'), it.next()];
    log('t5', JSON.stringify(await Promise.all(ps)));
  }

  // 6) return() before any next() at all.
  {
    async function* g() {
      try { yield 1; yield 2; } finally { log('  t6 finally'); }
    }
    const it = g();
    const p1 = it.return('EARLY');
    const p2 = it.next();
    log('t6', JSON.stringify(await Promise.all([p1, p2])));
  }

  // 7) for-await consuming, with the ordering visible through side effects.
  {
    const order = [];
    async function* g() {
      order.push('body-start');
      yield 1;
      order.push('after-1');
      await sleep(2);
      yield 2;
      order.push('after-2');
    }
    const it = g();
    const a = it.next();
    const b = it.next();
    order.push('both-issued');
    await Promise.all([a, b]);
    log('t7', JSON.stringify(order));
  }
}

main().then(() => log('done'), (e) => log('MAIN REJECTED', String(e)));
