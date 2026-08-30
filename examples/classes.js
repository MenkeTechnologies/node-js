// ES6 classes: inheritance, super, static members, getters/setters, instanceof.
class Shape {
  constructor(name) {
    this.name = name;
  }
  describe() {
    return `${this.name} with area ${this.area()}`;
  }
  area() {
    return 0;
  }
  static kinds() {
    return ["circle", "rect", "square"];
  }
}

class Rectangle extends Shape {
  #w;
  #h;
  constructor(w, h) {
    super("rectangle");
    this.#w = w;
    this.#h = h;
  }
  area() {
    return this.#w * this.#h;
  }
  get perimeter() {
    return 2 * (this.#w + this.#h);
  }
  set width(v) {
    this.#w = v;
  }
}

class Square extends Rectangle {
  constructor(s) {
    super(s, s);
    this.name = "square";
  }
  describe() {
    return "square: " + super.describe();
  }
}

const r = new Rectangle(3, 4);
console.log(r.describe());
console.log("perimeter", r.perimeter);
r.width = 5;
console.log("area after resize", r.area());

const sq = new Square(6);
console.log(sq.describe());
console.log("area", sq.area());

console.log("instanceof Rectangle", sq instanceof Rectangle);
console.log("instanceof Shape", sq instanceof Shape);
console.log("instanceof Object", sq instanceof Object);
console.log("static", Shape.kinds());
console.log("ctor name", sq.constructor.name);

// `super` in a STATIC method resolves against the parent CONSTRUCTOR, not the
// parent's prototype — the two home objects differ, and the class name alone
// cannot tell a static method from an instance one since both carry the same
// one. Always taking the prototype meant a static `super.x()` found nothing and
// then threw trying to call it.
class SBase {
  greet() { return "base-instance"; }
  static make() { return "base-static"; }
  static get tag() { return "base-tag"; }
}
class SDerived extends SBase {
  greet() { return "derived+" + super.greet(); }
  static make() { return "derived+" + super.make(); }
  static readTag() { return super.tag; }
}
console.log("inst-super", new SDerived().greet());
console.log("stat-super", SDerived.make());
console.log("stat-getter", SDerived.readTag());
// Reaching the parent explicitly always worked and must keep working.
class SExplicit extends SBase { static make() { return "explicit+" + SBase.make(); } }
console.log("explicit  ", SExplicit.make(), Object.getPrototypeOf(SDerived) === SBase);

// A bound function's `length` is the target's less the arguments already bound
// (20.2.3.2), floored at 0. Every bound function used to report 0, which breaks
// arity dispatch — express selects error-handling middleware with
// `fn.length === 4`, so a bound handler was never recognised as one.
function arity3(a, b, c) {}
console.log("arity     ", arity3.length, arity3.bind(null).length, arity3.bind(null, 1).length);
console.log("arity2    ", arity3.bind(null, 1, 2).length, arity3.bind(null, 1, 2, 3, 4).length);
console.log("chained   ", arity3.bind(null, 1).bind(null, 2).length, arity3.bind(null).name);

// `super[expr]` — the COMPUTED twin of `super.m`. Only the dotted form was
// compiled; the computed one fell through to the ordinary path, which compiled
// `super` as a value and then read or called against `undefined`. Both the
// call and the plain read were affected, on instance and static methods alike.
class SuperBase {
  greet() { return "base"; }
  get label() { return "base-label"; }
  static make() { return "base-make"; }
  static get tag() { return "base-tag"; }
}
class SuperChild extends SuperBase {
  greet() { return "child+" + super["greet"](); }
  viaVariable() { const key = "greet"; return "var+" + super[key](); }
  readLabel() { return super["label"]; }
  withArgs() { return super["greet"](1, 2); }
  bothForms() { return [super["greet"](), super.greet()].join("|"); }
  static make() { return "child+" + super["make"](); }
  static readTag() { return super["tag"]; }
}
const child = new SuperChild();
console.log("computed", child.greet(), child.viaVariable());
console.log("read    ", child.readLabel(), child.withArgs());
console.log("static  ", SuperChild.make(), SuperChild.readTag());
// The dotted form must keep working, and the two must agree.
console.log("agree   ", child.bothForms());

// `super` in an OBJECT LITERAL method. Only a home CLASS was tracked, so a
// shorthand method in a literal had no parent to resolve against and reported
// the method missing. The literal is its method's [[HomeObject]], and `super`
// reads through that object's prototype — which is why reassigning the
// prototype afterwards changes what `super` finds.
const litBase = { greet() { return "base"; }, get label() { return "bl"; }, tag: "bt" };
const literal = {
  __proto__: litBase,
  greet() { return "own+" + super.greet(); },
  computed() { return super["greet"](); },
  readGetter() { return super.label; },
  readData() { return super.tag; },
  *gen() { yield super.greet(); },
  ["dyn" + "amic"]() { return super.greet(); },
};
console.log("literal ", literal.greet(), literal.computed(), literal.dynamic());
console.log("lit-read", literal.readGetter(), literal.readData(), [...literal.gen()].join(","));
const assigned = { m() { return super.greet(); } };
Object.setPrototypeOf(assigned, litBase);
console.log("setproto", assigned.m());
// Only a METHOD DEFINITION gets a home object; a plain function-valued
// property is an ordinary property and cannot use `super` at all.
const plainProp = { __proto__: litBase, m: function () { return typeof this.greet; } };
console.log("plainfn ", plainProp.m());

// An ARROW has no `super` of its own and uses the enclosing METHOD's, exactly
// as it uses the enclosing `this`. Nothing was captured, so `super` inside an
// arrow failed — in a CLASS method as well as a literal.
class ArrowBase { m() { return "AB"; } static s() { return "AS"; } get g() { return "AG"; } }
class ArrowChild extends ArrowBase {
  m() { const f = () => super.m(); return "C+" + f(); }
  deep() { const f = () => () => super.m(); return f()(); }
  callback() { return [1].map(() => super.m())[0]; }
  getter() { const f = () => super.g; return f(); }
  static s() { const f = () => super.s(); return "CS+" + f(); }
}
console.log("arrow   ", new ArrowChild().m(), new ArrowChild().deep(), new ArrowChild().callback());
console.log("arrow2  ", new ArrowChild().getter(), ArrowChild.s());
const litArrow = { __proto__: litBase, m() { const f = () => super.greet(); return f(); } };
console.log("litarrow", litArrow.m(), [1].map(() => 0)[0]);
// `this` inside such an arrow is still the instance, and a non-arrow function
// inside a method does NOT inherit `super`.
class ThisCheck extends ArrowBase { m() { const f = () => this.constructor.name; return f(); } }
console.log("this    ", new ThisCheck().m());
// [[HomeObject]] is fixed where the method is DEFINED. A method that merely
// arrives as a value keeps the home it was defined with — so copying one into
// another literal must not rebind it, nor change what the original resolves.
// Stamping every method-VALUED property, rather than every method DEFINITION,
// got this wrong in both directions at once.
const homeA = { m() { return "A"; } };
const homeB = { m() { return "B"; } };
const definedInA = { __proto__: homeA, m() { return super.m(); } };
const copiedIntoB = { __proto__: homeB, m: definedInA.m };
console.log("home-def", definedInA.m(), copiedIntoB.m(), definedInA.m());
const assignedLater = { __proto__: homeB };
assignedLater.m = definedInA.m;
console.log("home-asn", assignedLater.m(), definedInA.m());
