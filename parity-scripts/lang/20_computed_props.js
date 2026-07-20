// Computed property names and dynamic keys.
const prefix = "user";
const id = 42;

const obj = {
  [prefix + "Id"]: id,
  [`${prefix}Name`]: "Ada",
  [`is${prefix[0].toUpperCase() + prefix.slice(1)}`]: true,
};
console.log(JSON.stringify(obj));

// Computed keys from array.
const keys = ["a", "b", "c"];
const mapped = {};
keys.forEach((k, i) => {
  mapped[k] = i * 10;
});
console.log(JSON.stringify(mapped));

// Computed method names.
const action = "run";
const machine = {
  [action]() {
    return "running";
  },
  [`${action}Fast`]() {
    return "running fast";
  },
};
console.log(machine.run(), machine.runFast());

// Symbol as computed key.
const tag = Symbol("tag");
const tagged = { [tag]: "secret", visible: "shown" };
console.log("visible keys:", Object.keys(tagged).join(","));
console.log("symbol value:", tagged[tag]);

// Build lookup table via computed props from entries.
const entries = [["x", 1], ["y", 2], ["z", 3]];
const table = entries.reduce((acc, [k, v]) => ({ ...acc, [k]: v * v }), {});
console.log(JSON.stringify(table));
