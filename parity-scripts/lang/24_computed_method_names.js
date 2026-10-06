// SetFunctionName for class members under a COMPUTED key: a symbol key gives
// `[description]` (or '' for a description-less symbol), a string key itself,
// and an accessor its `get `/`set ` prefix.
const anon = Symbol();
let n = 0;
const key = () => "k" + ++n;
class K {
  static [Symbol.iterator]() {}
  [Symbol.for("reg")]() {}
  get [Symbol.split]() { return 1; }
  set [Symbol.split](v) {}
  [key()]() {}
  static get [key()]() { return 2; }
  *[Symbol("gen")]() { yield 1; }
  async [anon]() {}
  [1 + 1]() {}
  static [`t${"pl"}`] = function () {};
}
const P = K.prototype;
console.log(K[Symbol.iterator].name, P[Symbol.for("reg")].name);
const split = Object.getOwnPropertyDescriptor(P, Symbol.split);
console.log(split.get.name, split.set.name, P.k1.name, Object.getOwnPropertyDescriptor(K, "k2").get.name);
const gen = Object.getOwnPropertySymbols(P).find((s) => s.description === "gen");
console.log(P[gen].name, JSON.stringify(P[anon].name), P[2].name, K.tpl.name, n);
console.log(Object.getOwnPropertyNames(P), Object.getOwnPropertyNames(K));
console.log(P[Symbol.for("reg")].hasOwnProperty("prototype"), [...new K()[gen]()]);
// `super` still resolves inside a computed-key method.
class B { m() { return "base"; } }
class D extends B { ["m"]() { return "derived+" + super.m(); } static [Symbol.hasInstance](x) { return x === 1; } }
console.log(new D().m(), 1 instanceof D, D[Symbol.hasInstance].name);
