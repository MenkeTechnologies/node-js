class A { get v() { return this._v; } set v(x) { this._v = x; } static set sv(x) { A._sv = x; } }
class B extends A {
  set v(x) { super.v = x + '!'; }
  get v() { return 'B>' + super.v; }
  inc() { super.v += 5; super['v'] *= 2; return this._v; }
  upd() { super.v++; return this._v; }
  setNew() { super.fresh = 1; return Object.keys(this).join(); }
  static st() { super.sv = 'S'; return A._sv; }
}
const b = new B(); b._v = 1;
console.log(b.inc(), b.upd(), b.setNew(), B.st());
b.v = 'x'; console.log(b._v, b.v);
const o = { __proto__: { set p(x) { this.got = x; } }, m() { super.p = 7; return this.got; } };
console.log(o.m());
class R { get ro() { return 1; } }
class S extends R { w() { 'use strict'; try { super.ro = 2; } catch (e) { return e.constructor.name; } return 'no throw'; } }
console.log(new S().w());
