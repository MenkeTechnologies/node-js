const log = [];
const key = (n) => { log.push('key:' + n); return n; };
class C { [key('a')] = 1; static [key('s')] = 2; [key('b')]() {} static { log.push('blk'); } static [key('t')] = log.push('init t'); get [key('g')]() { return 1; } }
console.log(log.join());
console.log(Object.getOwnPropertyNames(new C()).join(), Object.getOwnPropertyNames(C).join());
class A { static x = 1; static y = this.x + 1; static f = () => this.y; static g() { return this.x; } static { this.z = this.f() + 1; } static #p = this.x + 100; static getP() { return A.#p; } static fn = function () {}; static #pf = () => 1; static pfn() { return A.#pf.name; } }
class B extends A { static x = 10; static { this.w = super.g() + this.z; } static s = super.g(); }
console.log(A.x, A.y, A.f(), A.z, B.x, B.y, B.z, B.w, B.s, A.getP(), A.fn.name, A.pfn());
class P { #x = 1; static read(o) { return o.#x; } static write(o) { o.#x = 2; } static call(o) { return o.#m(); } #m() { return 1; } }
const P1 = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } };
console.log(P1(() => P.read({})), P1(() => P.write({})), P1(() => P.call({})), P1(() => P.read(Object.create(new P()))), P1(() => P.read(null)), P1(() => P.read(new P())), P1(() => P.call(new P())));
const L = [];
class Base { constructor() { L.push('A:' + new.target.name); } }
class Der extends Base { f = L.push('Der.f'); constructor() { const g = () => super(); L.push('pre'); g(); L.push('post'); } }
new Der();
console.log(L.join(' | '));
