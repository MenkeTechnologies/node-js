// `fs.existsSync` given a wrongly-TYPED path answers false and raises the
// one-time DEP0187 DeprecationWarning (on the next tick, with a `warning`
// event); a string that is merely unusable (a NUL) raises nothing. node-js
// answered false silently.
const fs = require("fs");
process.on("warning", (w) => console.log("warning", w.name, w.code, w.message));
console.log("nul    ", fs.existsSync("a\0b"));
console.log("object ", fs.existsSync({}));
console.log("number ", fs.existsSync(5));
console.log("sync   ", "logged before the warning");
