// The [[Prototype]] chain of constructors: a NativeError constructor inherits
// from Error, a builtin function from Function.prototype, and isPrototypeOf
// walks the same chain Object.getPrototypeOf reports — a class's parent
// constructor included.
class A {}
class B extends A {}
class C extends B {}
class MyErr extends Error {}
const p = Object.getPrototypeOf;
console.log(p(RangeError) === Error, p(TypeError).name, p(AggregateError) === Error, p(Error) === Function.prototype);
console.log(p(Math.max) === Function.prototype, p(parseInt) === Function.prototype, p(p(Error)) === Object.prototype);
console.log(A.isPrototypeOf(C), B.isPrototypeOf(C), C.isPrototypeOf(A), Object.prototype.isPrototypeOf.call(A, B));
console.log(Error.isPrototypeOf(MyErr), Error.isPrototypeOf(RangeError), Error.isPrototypeOf(Error), Error.isPrototypeOf(() => 1));
console.log(Function.prototype.isPrototypeOf(A), Function.prototype.isPrototypeOf(RangeError), Object.prototype.isPrototypeOf(Error));
console.log(Object.prototype.isPrototypeOf(1), Object.prototype.isPrototypeOf({}), Array.prototype.isPrototypeOf([]), Object.prototype.isPrototypeOf(Object.create(null)));
