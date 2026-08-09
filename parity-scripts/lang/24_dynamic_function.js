// `new Function(...)` / `Function(...)`: the dynamic-function constructor.
//
// The two shapes below hold IDENTICAL inputs and demand DIFFERENT output, so no
// rule read off the arguments alone can satisfy both: the `Function`
// constructor joins parameters with "," and puts a newline before the closing
// paren of the parameter list, while `vm.compileFunction` (see stdlib/15_vm.js)
// joins with ", " and does not. Likewise the scope assertions: a body that ran
// in the caller's scope would see `hidden`, and a body that ran in an EMPTY
// scope would not see `shared` — only the global scope satisfies both.
const vm = require("vm");

const add = new Function("a", "b", "return a + b");
console.log(JSON.stringify(add.toString()));
console.log(add.name, add.length, typeof add, add(2, 3));

// Called without `new` — same operation.
const seven = Function("return 7");
console.log(JSON.stringify(seven.toString()), seven.name, seven.length, seven());

// No arguments at all: empty parameter list, empty body.
const empty = new Function();
console.log(JSON.stringify(empty.toString()), empty.name, empty.length, empty());

// A parameter fragment may itself carry several parameters.
const three = new Function("a,b", "c", "return [a, b, c].join('-')");
console.log(JSON.stringify(three.toString()), three.length, three(1, 2, 3));

// Reached through a function's own `.constructor` — how `get-intrinsic` gets it.
console.log((function () {}).constructor === Function);
console.log((function () {}).constructor("return 9")());

console.log(add instanceof Function, Object.getPrototypeOf(add) === Function.prototype);
console.log(Object.prototype.toString.call(add));

// `arguments` works inside a dynamic function body.
console.log(new Function("return arguments.length")(1, 2, 3));

// A bad body is a SyntaxError, not a crash.
try {
  new Function("return (");
  console.log("no throw");
} catch (e) {
  console.log(e.name, e instanceof SyntaxError);
}

// Scope: the body never sees the constructing function's locals, and its own
// `var` stays local to the body rather than leaking to the global object.
function build() {
  const hidden = "caller-local";
  void hidden;
  return new Function("return typeof hidden");
}
console.log(build()());
const withVar = new Function("a", "var zz = 5; return zz + a");
console.log(withVar(1), typeof globalThis.zz);

// The same generator backs `vm.compileFunction`, with V8's other source shape.
const compiled = vm.compileFunction("return a + b", ["a", "b"]);
console.log(JSON.stringify(compiled.toString()), JSON.stringify(compiled.name), compiled.length);
console.log(compiled(4, 5));
