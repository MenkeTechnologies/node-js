// Promise resolution and async-iteration ORDERING. Each probe runs against a
// ruler of chained .then callbacks, so the interleaving encodes how many
// microtask ticks each construct costs.
const log = s => console.log(s);
const th = { then(res) { res('TH'); } };

new Promise(r => r(th)).then(v => log('N ' + v));
Promise.resolve().then(() => th).then(v => log('T ' + v));
Promise.resolve().then(() => Promise.resolve('P')).then(v => log('C ' + v));
(async () => { log('a0'); log('a1 ' + await th); log('a2 ' + await Promise.resolve('R')); })();

(async function* () { yield Promise.resolve('g1'); yield 'g2'; })
  && (async () => { for await (const v of (async function* () { yield Promise.resolve('g1'); yield 'g2'; })()) log('G ' + v); log('Gdone'); })();
(async () => { for await (const v of [Promise.resolve('l1'), 'l2']) log('L ' + v); log('Ldone'); })();

const it = (async function* () { yield Promise.resolve('q'); })();
it.next().then(s => log('Q1 ' + s.value + ',' + s.done));
it.next().then(s => log('Q2 ' + s.value + ',' + s.done));

process.on('unhandledRejection', e => log('UR ' + e.message));
Promise.reject(new Error('unwatched'));
Promise.reject(new Error('watched')).catch(e => log('CAUGHT ' + e.message));

let p = Promise.resolve();
for (let i = 1; i <= 12; i++) { const n = i; p = p.then(() => log('p' + n)); }
