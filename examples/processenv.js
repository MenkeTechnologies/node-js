// `process.env` and a few process properties. Nothing machine-specific is
// printed here — no pid, cwd, platform or version — only behaviour that is the
// same wherever this runs.

// Every environment value is a STRING. Assigning a number stored the number,
// so `process.env.PORT + 1` added where a real process concatenates.
process.env.T_NUM = 5;
console.log("num     ", process.env.T_NUM, typeof process.env.T_NUM, process.env.T_NUM + 1);
process.env.T_BOOL = true;
process.env.T_UND = undefined;
process.env.T_NUL = null;
console.log("coerced ", process.env.T_BOOL, process.env.T_UND, process.env.T_NUL);
console.log("types   ", typeof process.env.T_BOOL, typeof process.env.T_UND, typeof process.env.T_NUL);
process.env.T_OBJ = { a: 1 };
process.env.T_ARR = [1, 2];
console.log("objects ", process.env.T_OBJ, process.env.T_ARR, typeof process.env.T_ARR);
// Reading back, deleting, and enumerating all behave as on a plain object.
process.env.T_DEL = "gone";
delete process.env.T_DEL;
console.log("delete  ", String(process.env.T_DEL), "T_DEL" in process.env, "T_NUM" in process.env);
console.log("keys    ", Object.keys(process.env).includes("T_NUM"), Object.keys(process.env).some((k) => k.startsWith("@@")));

// `isTTY` is defined only when the fd IS a terminal — off a pipe the property
// is absent, not false. Defining it either way made `typeof` report "boolean",
// which is the exact check a library uses to decide whether to emit colour.
// This file always runs with its output captured, so both are undefined here.
console.log("tty     ", typeof process.stdout.isTTY, typeof process.stderr.isTTY, typeof process.stdout.fd);

// `process.release` carried nothing, so the common `process.release.name`
// probe threw on `undefined.name`.
console.log("release ", typeof process.release, process.release.name, process.release.name === "node");

// The rest of the surface these probes lean on, by shape rather than value.
console.log("shapes  ", Array.isArray(process.argv), typeof process.env, typeof process.pid, typeof process.cwd());
console.log("fns     ", typeof process.nextTick, typeof process.exit, typeof process.hrtime.bigint, typeof process.memoryUsage);
