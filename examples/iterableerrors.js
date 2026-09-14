// A not-iterable TypeError names the SOURCE EXPRESSION, not the value: node
// reports `o is not iterable` where this reported `{ a: 1 } is not iterable`.
// The machinery already existed for `is not a function` — a compile-time table
// of source texts keyed by op position — and handled only that suffix, and the
// iterator ops recorded nothing.
//
// Which wording node uses is not uniform, and none of it is derivable: each
// shape below was measured.
const show = (label, src) => {
  try { eval(src); console.log(label, "no throw"); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const o = { a: 1 }, n = 5, f = () => 5;

// `for-of` names the source, whatever its shape.
show("forof-ident  ", "for (const x of o) {}");
show("forof-number ", "for (const x of n) {}");
show("forof-member ", "for (const x of o.a) {}");
show("forof-index   ", "for (const x of o['a']) {}");
show("forof-literal", "for (const x of 5) {}");
show("forof-object ", "for (const x of {}) {}");
// …except a CALL, where either half could be at fault and V8 says so.
show("forof-call   ", "for (const x of f()) {}");
show("forof-anon   ", "for (const x of (() => 5)()) {}");
// `for await` names it too, and says ASYNC.
show("forawait     ", "(async () => { for await (const x of o) {} })().catch(e => console.log('forawait      ' + e.constructor.name + ': ' + e.message))");
// DESTRUCTURING names only a plain identifier or a literal. A member, an index,
// a call, a nested pattern and an assignment pattern all report the TYPE —
// with the property note, which the named form never carries.
show("destr-ident  ", "const [x] = o");
show("destr-number ", "const [x] = n");
show("destr-literal", "const [x] = 5");
show("destr-object ", "const [x] = {}");
show("destr-objfull", "const [x] = {a:1}");
show("destr-member ", "const [x] = o.a");
show("destr-call   ", "const [x] = (() => 5)()");
show("destr-nested ", "const [[w]] = [o]");
show("destr-assign ", "let y; [y] = o");
show("destr-param  ", "(function ([p]) {})(o)");
// SPREAD names its source. One op covers a whole array literal, so a name is
// recorded only when every spread in it would produce the same one.
show("spread-ident ", "[...o]");
show("spread-repeat", "[...o, ...o]");
show("spread-object", "[...{}]");
show("spread-objful", "[...{a:1}]");
// A non-empty OBJECT literal is the one callee shape V8 will not print from
// source — it is `{(intermediate value)}` here too, where an array literal is
// printed as written.
show("call-object  ", "({a:1})()");
show("call-empty   ", "({})()");
show("new-object   ", "new ({a:1})()");
