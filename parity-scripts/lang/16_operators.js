// Ternary, logical operators, short-circuit, coercion.
const values = [0, "", null, undefined, NaN, "hello", 42, [], {}];
for (const v of values) {
  console.log(JSON.stringify(v), "->", v ? "truthy" : "falsy");
}

// Short-circuit for defaults.
function greet(name) {
  return "Hi " + (name || "stranger");
}
console.log(greet("Ada"));
console.log(greet(""));

// && returns operand, not boolean.
const user = { role: "admin", perms: ["read", "write"] };
const perm = user && user.perms && user.perms[0];
console.log("perm:", perm);

// Chained ternary.
function grade(score) {
  return score >= 90 ? "A" : score >= 80 ? "B" : score >= 70 ? "C" : "F";
}
console.log([95, 85, 72, 50].map(grade).join(","));

// Numeric operators and precedence.
console.log(2 + 3 * 4);
console.log((2 + 3) * 4);
console.log(2 ** 3 ** 2); // right-assoc = 512
console.log(17 % 5, -17 % 5, 17 % -5);
console.log(7 / 2, Math.floor(7 / 2), 7 & 1);

// Bitwise.
console.log(5 & 3, 5 | 3, 5 ^ 3, ~5, 1 << 4, 256 >> 2);

// Comparison coercion (deterministic).
console.log(1 == "1", 1 === "1", null == undefined, null === undefined);
