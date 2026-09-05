// The ES2025 set operations and the string exotic object — two whole-value
// views the corpus did not pin.
//
// Every one of the seven set methods branches on the two sizes and walks the
// SMALLER side, which decides the result's order and which of the operand's
// methods runs; a set-like `{size, has, keys}` stands in for a real Set on the
// right of every one of them. The string half pins that a string PRIMITIVE
// answers the own-property questions its boxed form does.

const S = (...xs) => new Set(xs);

// Order: the receiver's elements first, then the operand's new ones.
console.log('union   ', [...S(3, 1).union(S(2, 1))]);

// Receiver larger than the operand: the OPERAND is walked, so its order wins.
console.log('inter-l ', [...S(3, 1, 2).intersection(S(2, 3))]);
console.log('inter-r ', [...S(2, 3).intersection(S(3, 1, 2))]);
console.log('diff    ', [...S(1, 2, 3).difference(S(2))]);
console.log('symdiff ', [...S(1).symmetricDifference(S(2, 1, 3))]);
console.log('preds   ', S(1, 2).isSubsetOf(S(1, 2, 3)), S(1, 2).isSupersetOf(S(2)), S(1).isDisjointFrom(S(2)));

// A set-like operand participates on the right of all seven.
const like = { size: 2, has: x => x === 1 || x === 9, keys: () => [1, 9][Symbol.iterator]() };
console.log('like    ', [...S(1, 2).union(like)], [...S(1, 2).intersection(like)], [...S(1, 2).difference(like)]);
console.log('like-p  ', S(1).isSubsetOf(like), S(1, 9, 3).isSupersetOf(like), S(2).isDisjointFrom(like));

// SameValueZero keys: `-0` is stored as `0`, `NaN` matches itself.
console.log('zeros   ', [...S(-0).union(S(0))], [...S(NaN).intersection(S(NaN))]);

// The result is always a plain Set, never the receiver's species.
class MySet extends Set {}
console.log('species ', new MySet([1]).union(S(2)).constructor.name);

// Each operand failure has its own diagnostic.
const boom = (f) => { try { f(); return 'no throw'; } catch (e) { return e.constructor.name + ': ' + e.message; } };
console.log('e-arg   ', boom(() => S(1).union([1, 2])));
console.log('e-nan   ', boom(() => S(1).union({ size: NaN, has() {}, keys() {} })));
console.log('e-neg   ', boom(() => S(1).union({ size: -1, has() {}, keys() {} })));
console.log('e-has   ', boom(() => S(1).union({ size: 1, has: 1, keys() {} })));
console.log('e-keys  ', boom(() => S(1).union({ size: 1, has() {}, keys: () => 5 })));

// ── a string as an object ──────────────────────────────────────────────────
// Its own keys are its code-unit indices plus a non-enumerable `length`, and
// the primitive must answer exactly as the boxed form does.
console.log('keys    ', Object.keys('ab'), Object.values('ab'));
console.log('entries ', Object.entries('ab'));
console.log('names   ', Object.getOwnPropertyNames('ab'), Object.getOwnPropertyNames(new String('ab')));
console.log('descr   ', Object.getOwnPropertyDescriptor('ab', 1));
console.log('length  ', Object.getOwnPropertyDescriptor('ab', 'length'));
console.log('assign  ', JSON.stringify(Object.assign({}, 'ab')), JSON.stringify({ ...'ab' }));
const forin = [];
for (const k in 'abc') forin.push(k);
console.log('for-in  ', forin);
console.log('empty   ', Object.keys(''), Object.getOwnPropertyNames(''));

// ── Symbol.toStringTag brands every conversion, not just the explicit call ──
const tagged = { [Symbol.toStringTag]: 'T' };
console.log('tag     ', String(tagged), `${tagged}`, tagged + '', tagged.toString());
class D { get [Symbol.toStringTag]() { return 'D'; } }
console.log('tag-get ', String(new D()), Object.prototype.toString.call(new D()));
console.log('exotics ', String(new Map()), String(Math), String({}));

// A proxy whose `get` trap hands back no callable `toString` refuses the
// conversion; a TRAPLESS proxy still brands as its target does.
console.log('proxy   ', boom(() => String(new Proxy({}, { get: () => undefined }))));
console.log('proxy-t ', String(new Proxy(new Map(), {})));
