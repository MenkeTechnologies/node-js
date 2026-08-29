// var hoisting, function hoisting, and the temporal dead zone.
//
// A `var` binding exists from the moment its scope is entered, so a read before
// the declaration is `undefined` rather than an error — which is exactly what
// separates it from `let`, where the same read throws.

// Read before the declaration: the binding is there, the value is not.
console.log(typeof v, v);
var v = 1;
console.log(v);

// The same inside a function, where the scope is the call rather than the file.
function readsEarly() {
  const before = inner;
  var inner = 2;
  return [before, inner];
}
console.log(readsEarly());

// A function declaration hoists whole, so it is callable above its own text.
console.log(callable());
function callable() {
  return "callable";
}

// A function declaration wins over a `var` of the same name: both hoist, and
// the function is the one that ends up bound.
function declWins() {
  var h;
  function h() {
    return 1;
  }
  return typeof h;
}
console.log(declWins());

// A bare `var x;` names a binding that already exists and must not reset it —
// most visibly when the name is a parameter.
function keepsParam(a) {
  var a;
  return a;
}
console.log(keepsParam(5), keepsParam());

// `var` ignores block scope; `let` does not.
function blockScope() {
  if (true) {
    var loose = 1;
    let tight = 2;
    void tight;
  }
  return [typeof loose, loose];
}
console.log(blockScope());

// Hoisting reaches through every block-scoped construct, since none of them
// scopes a `var`: loop heads, `switch` cases, and `try`/`catch`/`finally`.
function reaches() {
  const seen = [typeof i, typeof k, typeof sw, typeof t, typeof c, typeof fin];
  for (var i = 0; i < 1; i++) void i;
  for (var k in { a: 1 }) void k;
  switch (1) {
    case 1:
      var sw = "sw";
  }
  try {
    var t = "t";
    throw new Error("x");
  } catch (e) {
    var c = "c";
  } finally {
    var fin = "fin";
  }
  return [seen, [i, k, sw, t, c, fin]];
}
console.log(reaches());

// A destructuring `var` hoists every name it binds, defaults and rest included.
function destructured() {
  const before = [typeof d1, typeof d2, typeof d3, typeof rest];
  var { d1, missing: d2 = "dflt" } = { d1: "d1" };
  var [d3, ...rest] = [3, 4, 5];
  return [before, [d1, d2, d3, rest]];
}
console.log(destructured());

// The loop-capture difference: `var` has one binding for the whole loop, `let`
// makes a fresh one per iteration.
const fromVar = [];
for (var vi = 0; vi < 3; vi++) fromVar.push(() => vi);
const fromLet = [];
for (let li = 0; li < 3; li++) fromLet.push(() => li);
console.log(fromVar.map((f) => f()).join(","), fromLet.map((f) => f()).join(","));

// A `let` read before its declaration is a ReferenceError, not `undefined`.
try {
  dead;
  let dead = 1;
} catch (e) {
  console.log(e.constructor.name, e instanceof ReferenceError);
}

// `typeof` of a name that was never declared is "undefined" rather than a
// throw — the one read JS lets you make of an unresolvable name.
console.log(typeof neverDeclared);
