// Prototypes, Object.create, and instanceof.
function Animal(name) {
  this.name = name;
}
Animal.prototype.speak = function () {
  return `${this.name} makes a sound`;
};

function Dog(name) {
  Animal.call(this, name);
}
Dog.prototype = Object.create(Animal.prototype);
Dog.prototype.constructor = Dog;
Dog.prototype.speak = function () {
  return `${this.name} barks`;
};

const a = new Animal("generic");
const d = new Dog("Rex");
console.log(a.speak());
console.log(d.speak());
console.log("d instanceof Dog:", d instanceof Dog);
console.log("d instanceof Animal:", d instanceof Animal);
console.log("a instanceof Dog:", a instanceof Dog);

// Object.create with property descriptors.
const proto = {
  greet() {
    return `hi from ${this.id}`;
  },
};
const obj = Object.create(proto, {
  id: { value: 42, enumerable: true },
});
console.log(obj.greet());
console.log("has own greet:", obj.hasOwnProperty("greet"));
console.log("keys:", Object.keys(obj).join(","));
console.log("proto chain:", Object.getPrototypeOf(obj) === proto);
