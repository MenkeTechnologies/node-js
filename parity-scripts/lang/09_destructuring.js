// Destructuring: nested, defaults, rest, swapping.
const config = {
  server: { host: "localhost", port: 8080 },
  flags: ["a", "b", "c"],
  retries: 3,
};

const {
  server: { host, port },
  flags: [first, ...restFlags],
  timeout = 5000,
  retries,
} = config;

console.log(host, port);
console.log(first, restFlags.join(","));
console.log("timeout default:", timeout);
console.log("retries:", retries);

// Array destructuring with defaults and holes.
const [x, , z = 99, w = 100] = [1, 2, 3];
console.log(x, z, w);

// Swap without temp.
let m = 10;
let n = 20;
[m, n] = [n, m];
console.log("swapped:", m, n);

// Destructuring in function params.
function distance({ x: x1, y: y1 }, { x: x2, y: y2 }) {
  return Math.hypot(x2 - x1, y2 - y1);
}
console.log("dist:", distance({ x: 0, y: 0 }, { x: 3, y: 4 }));

// Nested with rest object.
const { server, ...others } = config;
console.log("others keys:", Object.keys(others).sort().join(","));
