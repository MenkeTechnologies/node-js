// Objects, destructuring, spread, and JSON.
const user = { name: "Ada", age: 36, roles: ["admin", "dev"] };
const { name, ...meta } = user;
console.log(name, meta);
const clone = { ...user, active: true };
console.log(Object.keys(clone), Object.values(meta));
console.log(JSON.stringify(clone));
const parsed = JSON.parse('{"a":1,"b":[2,3]}');
console.log(parsed.a, parsed.b);
