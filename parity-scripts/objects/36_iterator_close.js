const log = [];
const mk = (n, o = {}) => ({ [Symbol.iterator]() { let i = 0; return { next() { log.push('next' + i); return i < n ? { value: i++, done: false } : { value: undefined, done: true }; }, return(v) { log.push('return'); if (o.throwReturn) throw new Error('rt'); return o.bad ? 1 : { done: true }; } }; } });
let tn = 0;
const T = (f) => { log.length = 0; let r; try { r = f(); } catch (e) { r = 'E:' + e.message; } console.log(++tn, r, log.join(' ')); };
T(() => { for (const x of mk(3)) { throw new Error('body'); } });
T(() => { for (const x of mk(3, { throwReturn: true })) { throw new Error('body'); } });
T(() => { for (const x of mk(3, { throwReturn: true })) { break; } });
T(() => { for (const x of mk(3, { bad: true })) { break; } });
T(() => { const [a] = mk(3); return a; });
T(() => { const [a] = mk(3, { throwReturn: true }); return a; });
T(() => { const [a, b, c, d] = mk(3, { throwReturn: true }); return a; });
T(() => { const [a, ...r] = mk(3, { throwReturn: true }); return a; });
T(() => { const [{ x }] = [null]; });
T(() => { const [a] = mk(1, { bad: true }); return a; });
T(() => { function* g() { try { yield 1; yield 2; } finally { log.push('gfin'); } } const [a] = g(); return a; });
T(() => { function* g() { try { yield 1; } finally { log.push('gfin'); } } for (const x of g()) { throw new Error('b'); } });
T(() => { return Array.from(mk(3), (x) => { if (x == 1) throw new Error('m'); return x; }); });
T(() => { const [x, y] = mk(2, { throwReturn: true }); return x + y; });
T(() => { const it = { [Symbol.iterator]: () => ({}) }; const [a] = it; });
T(() => { const it = { [Symbol.iterator]: () => ({ next: () => 1 }) }; for (const x of it); });
function* g1() { try { yield* mk(3, { throwReturn: false }); } finally { log.push('g-finally'); } }
function* g2() { try { yield* mk(3, { throwReturn: true }); } finally { log.push('g-finally'); } }
for (const [name, g, how] of [['ret', g1, (it) => it.return(9)], ['retThrow', g2, (it) => it.return(9)], ['thr', g1, (it) => it.throw(new Error('t'))], ['thrThrow', g2, (it) => it.throw(new Error('t'))]]) {
  T(() => { log.length = 0; const it = g(); it.next(); return JSON.stringify(how(it)); });
}
T(() => { function* inner() { try { yield 1; yield 2; } catch (e) { yield 'caught ' + e; } } function* outer() { const r = yield* inner(); yield 'r:' + r; } const it = outer(); it.next(); return JSON.stringify([it.throw('X'), it.next(), it.next()]); });
