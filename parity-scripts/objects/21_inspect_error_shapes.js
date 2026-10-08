// util.inspect of errors (node's formatError): a stack with no frames is
// bracketed, `cause` and an array `errors` print as hidden keys, and a
// multi-line stack forces the multi-line brace form, re-indented when nested.
const withStack = (e, s) => { e.stack = s; return e; };
console.log(withStack(new Error("x", { cause: 5 }), "Error: x"));
console.log(withStack(new AggregateError([1, "x"], "m"), "AggregateError: m"));
console.log(withStack(new AggregateError([1], "m"), "AggregateError: m\n    at foo"));
const c = withStack(new Error("y"), "Error: y\n    at bar");
c.code = "Q";
console.log(c, [c], { c });
console.log(withStack(new Error("no frames"), "something else"));
console.log(withStack(new Error("m1"), ""));
console.log([withStack(new TypeError("t"), "TypeError: t")]);
const n = withStack(new Error("outer", { cause: withStack(new Error("inner"), "Error: inner") }), "Error: outer");
console.log(n);
