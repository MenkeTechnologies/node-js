// `new C(...args)` did not spread at all: each argument was compiled as an
// ordinary expression, and a spread there evaluates to the SPREAD OBJECT — so
// the array arrived as a single argument and `new Date(...[2020, 0, 1])` built
// an Invalid Date.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
class Two { constructor(a, b) { this.a = a; this.b = b; } }
function Legacy(a, b) { this.s = [a, b]; }
show("new-spread  ", () => new Two(...[1, 2]));
show("new-mixed   ", () => new Two(0, ...[1, 2]));
show("new-legacy  ", () => new Legacy(...[3, 4]));
show("new-native   ", () => new Date(...[2020, 0, 1]).getFullYear());
show("new-iterable ", () => new Two(...new Set([5, 6])));
show("new-string   ", () => new Two(..."ab"));
show("new-empty    ", () => new Two(...[]));
show("new-trailing ", () => new Two(...[1], 9));

// `Function.prototype.apply` takes an ARRAY-LIKE, not an iterable
// (CreateListFromArrayLike) — the same hole `Reflect.apply` had. An
// `arguments` object is the shape this is written for.
show("apply-like   ", () => Math.max.apply(null, { length: 2, 0: 1, 1: 5 }));
show("apply-args   ", () => { function f() { return Math.max.apply(null, arguments); } return f(3, 9, 2); });
show("apply-user   ", () => { function f(a, b) { return [a, b]; } return f.apply(null, { length: 2, 0: "x", 1: "y" }); });
show("apply-array  ", () => Math.max.apply(null, [1, 3, 2]));
// A nullish list means NO arguments; a primitive is a TypeError.
show("apply-null   ", () => Math.max.apply(null, null));
show("apply-none   ", () => Math.max.apply(null));
show("apply-number ", () => Math.max.apply(null, 1));
show("apply-string ", () => Math.max.apply(null, "12"));

// A spread in a CALL argument list reports a non-iterable differently from one
// in an array literal: node names the missing PROTOCOL there and the VALUE
// here. A nullish spread names the value in both.
show("call-object  ", () => Math.max(...{ length: 2 }));
show("call-number  ", () => Math.max(...5));
show("call-null    ", () => Math.max(...null));
show("call-undef   ", () => Math.max(...undefined));
show("new-object   ", () => new Two(...{ length: 1, 0: 2 }));
show("array-number ", () => [...5]);
show("destructure  ", () => { const [a] = {}; return a; });

// Every spread that WORKS must keep working.
show("ok-array     ", () => Math.max(...[1, 5, 2]));
show("ok-string    ", () => [..."ab"]);
show("ok-set       ", () => [...new Set([1, 2])]);
show("ok-map       ", () => [...new Map([[1, 2]])]);
show("ok-generator ", () => { function* g() { yield 1; yield 2; } return [...g()]; });
show("ok-arguments ", () => { function f() { return [...arguments]; } return f(1, 2); });
show("ok-custom    ", () => { const o = { *[Symbol.iterator]() { yield 7; } }; return [...o]; });
show("ok-method    ", () => ({ m(...a) { return a; } }).m(...[1, 2]));
show("ok-mixed     ", () => { function f(...a) { return a; } return f(0, ...[1, 2], 3); });
