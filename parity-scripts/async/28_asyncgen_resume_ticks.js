// How many microtask ticks each async-generator resumption costs, measured
// against a ruler of chained .then callbacks.
const log = (s) => console.log(s);

function ruler(tag, n) {
  let p = Promise.resolve();
  for (let i = 1; i <= n; i++) { const k = i; p = p.then(() => log(tag + k)); }
}

// A) .next() then .next(): second resumption is a NORMAL completion.
(() => {
  async function* g() { yield 'v'; }
  const it = g();
  it.next().then((s) => log('A.q1 ' + s.value + ',' + s.done));
  it.next().then((s) => log('A.q2 ' + s.value + ',' + s.done));
  ruler('A.p', 6);
})();

setTimeout(() => {
  // B) .next() then .return(): resumption is a RETURN completion at a yield.
  async function* g() { try { yield 'v'; } finally { log('B.finally'); } }
  const it = g();
  it.next().then((s) => log('B.q1 ' + s.value + ',' + s.done));
  it.return('R').then((s) => log('B.q2 ' + s.value + ',' + s.done));
  ruler('B.p', 6);
}, 0);

setTimeout(() => {
  // C) .next() then .throw(): resumption is a THROW completion at a yield.
  async function* g() {
    try { yield 'v'; } catch (e) { log('C.caught ' + e); yield 'after'; }
  }
  const it = g();
  it.next().then((s) => log('C.q1 ' + s.value + ',' + s.done));
  it.throw('E').then((s) => log('C.q2 ' + s.value + ',' + s.done));
  ruler('C.p', 6);
}, 10);

setTimeout(() => {
  // D) .return() on a generator that never started (no yield suspension).
  async function* g() { try { yield 'v'; } finally { log('D.finally'); } }
  const it = g();
  it.return('R').then((s) => log('D.q1 ' + s.value + ',' + s.done));
  ruler('D.p', 6);
}, 20);

setTimeout(() => {
  // E) .return() on an already-completed generator.
  async function* g() { yield 'v'; }
  const it = g();
  it.next().then(() => {
    it.next().then(() => {
      it.return('R').then((s) => log('E.q ' + s.value + ',' + s.done));
      ruler('E.p', 6);
    });
  });
}, 30);
