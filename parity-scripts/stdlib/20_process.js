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

// Everything above is an EXISTENCE check: `typeof x === "string"` is satisfied
// by any string, so a `process.argv[0]` of `""` and a `cwd()` of `"nope"` pass
// it. Each line below re-derives the same read from a second, independent
// source, so the two have to agree on a VALUE rather than merely on a type.
const path = require("path");
console.log(process.argv.length === 2); // `node <file>`, no script args
console.log(process.argv[0] === process.execPath);
console.log(process.argv[1] === path.resolve(process.argv[1]));
console.log(path.basename(process.argv[1]));
console.log(process.cwd() === path.resolve(process.cwd()), path.isAbsolute(process.cwd()));
console.log(Number.isInteger(process.pid), process.pid !== process.ppid);
console.log(process.version[0] === "v", /^v\d+\.\d+\.\d+/.test(process.version));
console.log(process.version.slice(1) === process.versions.node);
// The harness pins TZ/LANG/LC_ALL, so this is a fixed value on both sides.
console.log(process.env.TZ, Object.keys(process.env).includes("TZ"));

// IDENTITY, which no `typeof` can see. Each of these was `false` here: every
// read of `process.env`/`process.argv`/`process.stdout` rebuilt the object, so
// `process.env.NODE_ENV = "x"` wrote to a throwaway and read back `undefined` —
// while every `typeof` line above stayed green throughout.
console.log(process.env === process.env, process.argv === process.argv);
console.log(process.stdout === process.stdout, process.versions === process.versions);
process.env.NODE_JS_PARITY_PROBE = "set";
console.log(process.env.NODE_JS_PARITY_PROBE);
console.log(process === globalThis.process);
// `typeof f === "function"` does not mean `f()` works: `process.hrtime.bigint`
// answered "function" while calling it threw.
console.log(typeof process.hrtime.bigint === "function", typeof process.hrtime.bigint() === "bigint");
console.log(process.hrtime().length === 2, Number.isInteger(process.hrtime()[0]));
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
