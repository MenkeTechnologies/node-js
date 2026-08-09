// `continue` must run the loop's UPDATE clause. Landing on the test instead
// skips it (silent wrong answer or a hang), and is per-loop-form.
const o = [];
for (let i = 0; i < 6; i++) { if (i % 2) continue; o.push('f' + i); }
o.push('|');
for (let i = 0, j = 10; i < 4; i++, j--) { if (i === 1) continue; o.push(i + ':' + j); }
o.push('|');
let k = 0; while (k < 6) { k++; if (k % 2) continue; o.push('w' + k); }
o.push('|');
let d = 0; do { d++; if (d % 2) continue; o.push('d' + d); } while (d < 6);
o.push('|');
for (const v of [1, 2, 3, 4]) { if (v % 2) continue; o.push('of' + v); }
o.push('|');
for (const key in { a: 1, b: 2, c: 3 }) { if (key === 'b') continue; o.push('in' + key); }
o.push('|');
// continue crossing a switch, and from inside a try, per form
for (let i = 0; i < 4; i++) { switch (i % 2) { case 1: continue; } o.push('sw' + i); }
o.push('|');
for (let i = 0; i < 4; i++) { try { if (i % 2) continue; } finally { o.push('t' + i); } o.push('b' + i); }
o.push('|');
let m = 0; while (m < 4) { m++; try { if (m % 2) continue; } finally { o.push('W' + m); } }
o.push('|');
let q = 0; do { q++; try { if (q % 2) continue; } finally { o.push('D' + q); } } while (q < 4);
o.push('|');
outer: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j === 1) continue outer; o.push('L' + i + j); } }
console.log(o.join(','));
