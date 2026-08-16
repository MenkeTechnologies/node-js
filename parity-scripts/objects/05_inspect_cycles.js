const c = { a: 1 }; c.c = c; console.log(c);
const arr = [1]; arr.push(arr); console.log(arr);
const m = new Map(); m.set('m', m); console.log(m);
const s = new Set(); s.add(s); console.log(s);
// Two distinct cycle targets get distinct ids.
const p = { a: {} }; p.a.up = p; p.b = p; console.log(p);
// Mutual recursion between two objects.
const n1 = {}, n2 = {}; n1.n = n2; n2.n = n1; console.log(n1);
// A callable is marked the same way.
const f = function () {}; f.self = f; console.log(f);
// A repeated but ACYCLIC reference is not a cycle and gets no marker.
const shared = { s: 1 }; console.log([shared, shared]);
// The depth limit still applies to non-cyclic nesting.
console.log({ a: { b: { c: { d: 1 } } } });
