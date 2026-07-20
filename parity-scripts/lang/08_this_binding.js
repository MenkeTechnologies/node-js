// this binding: call, apply, bind, arrow functions.
const person = {
  name: "Ada",
  greet(greeting) {
    return `${greeting}, I am ${this.name}`;
  },
};

const other = { name: "Grace" };
console.log(person.greet("Hello"));
console.log(person.greet.call(other, "Hi"));
console.log(person.greet.apply(other, ["Hey"]));

const bound = person.greet.bind(other, "Bound");
console.log(bound());

// Arrow functions capture lexical this.
const counter = {
  count: 0,
  values: [1, 2, 3],
  sum() {
    let total = this.count;
    this.values.forEach((v) => {
      total += v; // arrow keeps this
    });
    return total;
  },
};
console.log("sum:", counter.sum());

// Partial application via bind.
function multiply(a, b, c) {
  return a * b * c;
}
const times2 = multiply.bind(null, 2);
console.log("times2(3,4):", times2(3, 4));
const times2and3 = multiply.bind(null, 2, 3);
console.log("times2and3(4):", times2and3(4));
