// A core module inspects as node prints it — a plain object of its members,
// `path.posix` inside `path` printing as `[Circular *1]` — and enumerates in
// node's order with its VALUE exports (`path.sep`, `os.EOL`, `fs.constants`)
// among the keys (both measured: `CORE_KEYS`). Every module printed as
// `Object [path] {}`, value exports were readable but not listed, and an
// anonymous builtin printed as `[Function: ]`.
const util = require("util");
console.log(util.inspect(require("path")));
console.log(util.inspect(require("querystring")));
console.log(util.inspect(require("string_decoder")));
console.log(util.inspect(require("path"), { depth: 0 }));
console.log(Object.keys(require("path")).join());
console.log(Object.keys(require("os")).join());
console.log(Object.keys(require("timers")).join());
console.log(["constants", "promises", "Stats"].map((k) => Object.keys(require("fs")).includes(k)).join());
console.log(require("os").freemem.name === "", String(require("util").inspect(require("os").freemem)).startsWith("[Function (anonymous)]"));
console.log(Object.keys(require("dns")).filter((k) => k !== "Resolver").join());
const seen = []; for (const k in require("os")) seen.push(k); console.log(seen.join());
