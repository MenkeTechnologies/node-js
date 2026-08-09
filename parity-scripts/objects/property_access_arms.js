// Property access across every heap-object variant.
//
// `get_property`/`set_property`/`call_method` pick their branch from an
// `ObjKind` tag rather than a clone of the receiver; this exercises one read and
// one write per variant so a mis-wired arm shows up as a byte divergence.
// Mutable aliasing is covered too: array handles are shared, never copied.
const p = (...a) => console.log(...a);

// ── Array ────────────────────────────────────────────────────────────────────
const a = [10, 20, 30];
p('arr', a.length, a[0], a[2], a[3], a[-1], a['1'], a['x']);
a.x = 'own';
p('arr.own', a.x, a.length);
a.length = 2; p('arr.trunc', a.length, JSON.stringify(a));
a.length = 4; p('arr.grow', a.length, JSON.stringify(a));
a[7] = 9; p('arr.sparse', a.length, JSON.stringify(a));
p('arr.iter', [...[1, 2, 3]], typeof a.map, typeof a.push);

// ── aliasing: a handle is shared, so every writer is visible to every reader ──
const s1 = [1]; const s2 = s1; s2.push(2); s2[0] = 99;
p('alias', s1, s2, s1 === s2, JSON.stringify(s1));
const nest = { v: [1] }; const ref = nest.v; ref.push(2);
p('alias.nested', JSON.stringify(nest), JSON.stringify(ref));
function mut(x) { x.push('in-fn'); } mut(s1); p('alias.arg', JSON.stringify(s1));

// push/unshift return the NEW length
const r = []; p('push.ret', r.push(1), r.push(2, 3), r.unshift(0), JSON.stringify(r));
p('pop/shift', r.pop(), r.shift(), JSON.stringify(r));

// ── Object ───────────────────────────────────────────────────────────────────
const o = { k: 1, u: undefined, 2: 'two' };
p('obj', o.k, o.u, 'u' in o, o.missing, o[2], o['2']);
p('obj.hasOwn', o.hasOwnProperty('k'), o.hasOwnProperty('u'), o.hasOwnProperty('nope'));
p('obj.toString', o.toString(), String(o));
o.__proto__ = { inherited: 'yes' }; p('obj.proto', o.inherited, o.__proto__.inherited);
const accessor = { _v: 5, get v() { return this._v * 2; }, set v(n) { this._v = n; } };
accessor.v = 21; p('accessor', accessor.v, accessor._v);

// ── String (BMP only — char vs UTF-16 indexing is a known divergence) ────────
const str = 'héllo';
p('str', str.length, str[0], str[1], str[9], str.toUpperCase(), typeof str.slice);

// ── Map / Set (Weak variants expose no `size`) ───────────────────────────────
const m = new Map([['a', 1]]); const st = new Set([1, 2, 2]);
p('map/set', m.size, st.size, m.get('a'), st.has(2), typeof m.forEach);
p('weak.size', new WeakMap().size, new WeakSet().size);

// ── Symbol / BigInt / RegExp ─────────────────────────────────────────────────
const sym = Symbol('desc');
p('symbol', sym.description, sym.toString(), Symbol().description);
const bi = 123456789012345678901234567890n;
p('bigint', bi.toString(), (2n ** 64n).toString(), typeof bi.valueOf);
const re = /a(b)c/gi;
p('regexp', re.source, re.flags, re.global, re.ignoreCase, re.multiline,
  re.dotAll, re.sticky, re.unicode, re.lastIndex);
re.lastIndex = 3; p('regexp.li', re.lastIndex, /x/.test('x'));

// ── Function / Class / bound ─────────────────────────────────────────────────
function f(x, y) { return x + y; }
class C { static s = 1; static sm() { return 'sm'; } m() { return 'm'; } }
const bf = f.bind(null, 1);
p('fn', f.name, f.length, bf.name, bf(2), C.s, C.sm(), new C().m());
p('fn.callapply', f.call(null, 1, 2), f.apply(null, [3, 4]));

// ── Generator / Promise / Iterator ───────────────────────────────────────────
function* g() { yield 1; yield 2; }
const gi = g();
p('gen', typeof gi.next, gi.next().value, [...g()]);
const it = [1, 2][Symbol.iterator]();
p('iter', it.next().value, typeof it.next);
p('promise', typeof Promise.resolve(1).then, typeof Promise.resolve(1).catch);

// ── Builtin namespaces ───────────────────────────────────────────────────────
p('builtin', Math.max(1, 2), typeof JSON.parse, Number.MAX_SAFE_INTEGER,
  typeof console.log, Array.isArray([]), Object.keys({ a: 1 }));

// ── Buffer: per-byte read AND write go through the hidden `@@bytes` array ────
const b = Buffer.from([1, 2, 3, 250]);
p('buf', b.length, b[0], b[3], b[99], b.toString('hex'));
b[1] = 300;   // wraps to 44
b[2] = -1;    // wraps to 255
b[10] = 7;    // past the end: dropped, never appended
p('buf.set', b[1], b[2], b.length, b[10], b.toString('hex'), JSON.stringify([...b]));
const bAlias = b; bAlias[0] = 77; p('buf.alias', b[0], b === bAlias);
p('buf.slice', b.slice(1, 3).toString('hex'), Buffer.concat([b, b]).length);

// ── TypedArray element get/set (same numeric-key interception) ──────────────
const ta = new Uint8Array(3); ta[0] = 260; ta[1] = 5;
p('ta', ta.length, ta[0], ta[1], ta[2], ta[9]);
const i32 = new Int32Array([1, -2]); p('ta.i32', i32[0], i32[1], i32.length);

// ── a nullish receiver still names the property it was reading ──────────────
try { null.x; } catch (e) { p('err.null', e.constructor.name, e.message); }
try { undefined.foo; } catch (e) { p('err.undef', e.message); }
