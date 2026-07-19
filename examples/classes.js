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
