// ES6 classes: inheritance, super, static, getters/setters.
class Shape {
  constructor(name) {
    this.name = name;
  }
  area() {
    return 0;
  }
  describe() {
    return `${this.name} with area ${this.area().toFixed(2)}`;
  }
  static compare(a, b) {
    return a.area() - b.area();
  }
}

class Circle extends Shape {
  constructor(r) {
    super("circle");
    this._r = r;
  }
  get radius() {
    return this._r;
  }
  set radius(v) {
    this._r = v < 0 ? 0 : v;
  }
  area() {
    return Math.PI * this._r * this._r;
  }
}

class Rectangle extends Shape {
  constructor(w, h) {
    super("rectangle");
    this.w = w;
    this.h = h;
  }
  area() {
    return this.w * this.h;
  }
}

const c = new Circle(2);
const r = new Rectangle(3, 4);
console.log(c.describe());
console.log(r.describe());
c.radius = -5;
console.log("clamped radius:", c.radius);
const shapes = [c, r, new Circle(1)];
shapes.sort(Shape.compare);
console.log(shapes.map((s) => s.name).join(","));
