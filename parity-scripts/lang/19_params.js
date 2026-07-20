// Default and rest parameters, argument handling.
function greet(name = "World", greeting = "Hello") {
  return `${greeting}, ${name}!`;
}
console.log(greet());
console.log(greet("Ada"));
console.log(greet("Ada", "Hi"));

// Rest parameters.
function sum(...nums) {
  return nums.reduce((a, b) => a + b, 0);
}
console.log(sum(1, 2, 3, 4, 5));
console.log(sum());

// Defaults referencing earlier params.
function makeRange(start, end = start + 10, step = 1) {
  const out = [];
  for (let i = start; i < end; i += step) out.push(i);
  return out;
}
console.log(makeRange(0).join(","));
console.log(makeRange(0, 6, 2).join(","));

// Mixed default + rest.
function format(sep = "-", ...parts) {
  return parts.join(sep);
}
console.log(format());
console.log(format(":", "a", "b", "c"));

// Default via function call.
function defaultVal() {
  return 100;
}
function withComputed(x = defaultVal()) {
  return x * 2;
}
console.log(withComputed(), withComputed(5));

// arguments object in non-arrow.
function count() {
  return arguments.length;
}
console.log("args:", count(1, 2, 3));
