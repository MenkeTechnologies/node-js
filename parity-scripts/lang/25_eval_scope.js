// Direct vs indirect `eval`, and the scope each one evaluates in.
//
// Every call below evaluates the SAME source string. A direct `eval` — the
// literal `eval(...)` call form — evaluates in the caller's scope; every other
// route to the same function value is an indirect eval and evaluates in the
// global scope (ECMA-262 19.2.1.1). So the identical string must produce
// OPPOSITE answers depending only on the call form, which no single scope rule
// can satisfy.
function probe() {
  const local = 42;
  void local;
  const src = "typeof local";
  return [
    eval(src), // direct   -> sees `local`
    (0, eval)(src), // indirect -> does not
    eval("typeof Array"), // direct   -> real globals visible
    (0, eval)("typeof Array"), // indirect -> likewise
  ].join(" ");
}
console.log(probe());

// Completion value, not just side effects.
console.log(eval("1 + 2"), (0, eval)("3 * 4"));
console.log(eval("var q = 5; q + 1"));

// A non-string argument is returned unchanged.
console.log(eval(7), eval(true), typeof eval(undefined), typeof eval({}));

// A direct eval can write the caller's binding.
function mutate() {
  let n = 1;
  eval("n = n + 10");
  return n;
}
console.log(mutate());

// A syntax error surfaces as a catchable SyntaxError.
try {
  eval("function (");
  console.log("no throw");
} catch (e) {
  console.log(e.name, e instanceof SyntaxError);
}

// An indirect eval's `var` does NOT land in the calling function.
function indirectVar() {
  (0, eval)("var fromIndirect = 'yes'");
  return typeof fromIndirect;
}
console.log(indirectVar());

// A user binding named `eval` shadows the intrinsic entirely.
function shadowed() {
  const evalFn = (s) => "shadow:" + s;
  return evalFn("x");
}
console.log(shadowed());
