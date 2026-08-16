class A {
  static x = 1;
  static #p = 3;
  static { this.viaThis = 5; }
  static { A.y = A.x + 1; A.viaPrivate = A.#p; }
  static m() { return 7; }
  static { A.viaMethod = A.m(); }
}
console.log(A.x, A.y, A.viaThis, A.viaPrivate, A.viaMethod);
// The block leaves NO property behind.
console.log(Object.getOwnPropertyNames(A).join(','), Object.keys(A).join(','));
// Block-scoped declarations inside are local to the block.
class Scoped { static { let v = 1; { let v = 2; void v; } Scoped.v = v; } }
console.log(Scoped.v);
// A class EXPRESSION and a nested class both work.
const C = class { static { this.n = 9; } };
class Outer { static { Outer.inner = class { static { this.deep = 1; } }; } }
console.log(C.n, Outer.inner.deep);
// `static` as an ordinary member name is unaffected.
class Named { static(){ return 'call'; } static static = 'field'; }
console.log(new Named().static(), Named.static);
