// Objects, destructuring, spread, and JSON.
const user = { name: "Ada", age: 36, roles: ["admin", "dev"] };
const { name, ...meta } = user;
console.log(name, meta);
const clone = { ...user, active: true };
console.log(Object.keys(clone), Object.values(meta));
console.log(JSON.stringify(clone));
const parsed = JSON.parse('{"a":1,"b":[2,3]}');
console.log(parsed.a, parsed.b);

// `util.inspect`'s depth option. `{ depth: null }` and `{ depth: Infinity }`
// both mean unlimited, which was stored as the maximum unsigned value — and the
// walk compares its indent against TWICE the depth, so doubling that overflowed
// and panicked the process. An abort, from an ordinary debugging call.
const util = require("util");
const deep = { a: { b: { c: { d: { e: 1 } } } } };
console.log("depth-def ", util.inspect(deep));
console.log("depth-0   ", util.inspect(deep, { depth: 0 }));
console.log("depth-1   ", util.inspect(deep, { depth: 1 }));
console.log("depth-null", util.inspect(deep, { depth: null }).replace(/\s+/g, " "));
console.log("depth-inf ", util.inspect(deep, { depth: Infinity }).replace(/\s+/g, " "));
// A NEGATIVE depth is legal and means "already past the limit". Only `null` and
// a non-finite depth mean unlimited; treating everything below zero as
// unlimited made this print the whole object instead of collapsing it.
console.log("depth-neg ", util.inspect(deep, { depth: -1 }), util.inspect(deep, { depth: -5 }));
console.log("depth-ninf", util.inspect(deep, { depth: -Infinity }), util.inspect([1, [2]], { depth: -1 }));
// Truncated toward zero, and the override lasts for the one call only.
console.log("depth-frac", util.inspect(deep, { depth: 1.9 }), util.inspect(deep));
