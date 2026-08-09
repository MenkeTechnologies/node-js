// break/continue/return/throw crossing loop, switch, labeled-block, try, catch
// and finally boundaries — including the cases where a finally's own abrupt
// completion replaces a pending one.
const out = [];

for (let i = 0; i < 5; i++) { if (i === 3) break; out.push(i); }
for (let i = 0; i < 5; i++) { if (i % 2 === 0) continue; out.push('c' + i); }
let w = 0; while (w < 5) { w++; if (w === 3) break; out.push('w' + w); }
let d = 0; do { d++; if (d === 2) continue; if (d === 4) break; out.push('d' + d); } while (d < 10);
for (const k in { a: 1, b: 2, c: 3 }) { if (k === 'b') continue; out.push('i' + k); }
for (const v of [1, 2, 3, 4]) { if (v === 3) break; out.push('o' + v); }
console.log(out.join(','));

const l = [];
outer: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j === 1) continue outer; if (i === 2) break outer; l.push(i + ':' + j); } }
A: for (const a of [1, 2, 3]) { for (const b of [1, 2, 3]) { if (b === 2) continue A; if (a === 3) break A; l.push(a + '-' + b); } }
L: { l.push('in'); if (true) break L; l.push('nope'); }
console.log(l.join(','));

function retFin() { try { return 1; } finally { return 3; } }
function throwFin() { try { throw new Error('boom'); } finally { return 2; } }
function nestedFin() { try { try { throw new Error('x'); } finally { return 'inner'; } } catch (e) { return 'caught ' + e.message; } }
console.log(retFin(), throwFin(), nestedFin());

const s = [];
switch (2) { case 2: try { s.push('a'); break; } finally { s.push('f'); } s.push('never'); }
SW: switch (1) { case 1: try { break SW; } finally { s.push('fin'); } }
BLK: { try { break BLK; } finally { s.push('blk'); } s.push('never'); }
for (let i = 0; i < 3; i++) { CASE: switch (i) { case 1: for (const x of [1, 2, 3]) { if (x === 2) break CASE; s.push('x' + x); } s.push('nope'); default: s.push('d' + i); } }
console.log(s.join(','));

const f = [];
for (let i = 0; i < 4; i++) { try { throw new Error('e' + i); } finally { f.push('F' + i); continue; } }
for (let i = 0; i < 4; i++) { try { break; } finally { f.push('g' + i); continue; } }
console.log(f.join(','));

function* gen() { try { yield 1; yield 2; } finally { f.push('genfin'); } }
const it = gen();
console.log(it.next().value, JSON.stringify(it.return(9)), JSON.stringify(it.next()), f.includes('genfin'));
