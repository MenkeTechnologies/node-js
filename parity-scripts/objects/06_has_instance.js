console.log(typeof Symbol.hasInstance, String(Symbol.hasInstance));
// A static method on a class, inherited down the `extends` chain.
class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }
class SubEven extends Even {}
console.log(2 instanceof Even, 3 instanceof Even, 4 instanceof SubEven, 5 instanceof SubEven);
// A plain (uncallable) object is a legal right-hand side once it defines it.
const oddish = { [Symbol.hasInstance](x) { return x > 2; } };
console.log([1, 2, 3, 4].filter((x) => x instanceof oddish).join(','));
// Defined on an ordinary function via defineProperty.
const f = function () {}; Object.defineProperty(f, Symbol.hasInstance, { value: () => true });
console.log(1 instanceof f);
// It THROWS through, it does not swallow.
class Boom { static [Symbol.hasInstance]() { throw new Error('boom'); } }
try { 1 instanceof Boom; } catch (e) { console.log(e.message); }
// GetMethod: undefined/null mean "absent", anything else uncallable is a TypeError.
for (const v of [1, 's', true, {}, null, undefined]) {
  const o = { [Symbol.hasInstance]: v };
  try { console.log(1 instanceof o); } catch (e) { console.log(e.constructor.name + ': ' + e.message); }
}
// The ordinary prototype walk is untouched.
class A {} class B extends A {}
console.log(new B() instanceof A, new A() instanceof B, [] instanceof Array, ({}) instanceof Object);
