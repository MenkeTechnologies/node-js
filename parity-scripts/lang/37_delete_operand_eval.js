// `delete` of a non-Reference still evaluates its operand, and an optional
// chain that short-circuits makes the whole `delete` evaluate to true.
let calls = 0;
function f() { calls++; return {}; }
console.log(delete f(), calls);
let o = null;
console.log(delete o?.a);
console.log(delete o?.a.b.c);
console.log(delete undefined?.[calls++], calls);
let p = { a: { b: 1 } };
console.log(delete p?.a.b, p.a);
console.log(delete p.a?.["b"], p);
let q = { x: 1 };
console.log(delete q?.x, q);
console.log(delete (1 + 2));
