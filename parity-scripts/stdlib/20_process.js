// process: deterministic-typeof introspection only (no version/pid strings).
console.log(process.argv.length >= 2);
console.log(typeof process.argv[0] === "string");
console.log(typeof process.platform === "string");
console.log(typeof process.arch === "string");
console.log(typeof process.cwd() === "string");
console.log(typeof process.pid === "number");
console.log(typeof process.version === "string");
console.log(typeof process.env === "object");
console.log(Array.isArray(process.argv));
console.log(typeof process.nextTick === "function");
console.log(typeof process.hrtime === "function");
console.log(process.platform === require("os").platform());
console.log(process.arch === require("os").arch());
// `typeof x === "undefined" || typeof x === "number"` was the original form
// here, and it is satisfied whether or not the property does anything — it was
// green while `process.exitCode` was stored, read back, and then ignored at
// exit. Kept, since an unset `exitCode` really is `undefined`, but no longer
// alone: the value has to survive a write and a clear.
console.log(typeof process.exitCode === "undefined" || typeof process.exitCode === "number");
console.log(process.exitCode);
process.exitCode = 5;
console.log(process.exitCode);
process.exitCode = null;
console.log(process.exitCode);
