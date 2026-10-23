// The `length` and `name` of the core modules' own functions. `length` read
// `undefined` for every one (the arity table covered the ECMAScript
// intrinsics only); it now comes from `src/core_arity.rs`, measured from the
// reference binary by `gen-core-arity`. Express-style arity dispatch
// (`fn.length === 4`) and `util.promisify`'s custom-argument handling read it.
const pick = {
  fs: ["readFileSync", "writeFile", "open", "createReadStream", "existsSync", "readSync"],
  path: ["join", "resolve", "relative", "parse"],
  util: ["format", "inspect", "promisify", "inherits", "deprecate", "isDeepStrictEqual", "parseArgs"],
  events: ["once", "getEventListeners"],
  os: ["platform", "cpus", "userInfo"],
  crypto: ["createHash", "randomBytes", "pbkdf2Sync"],
  child_process: ["spawn", "exec", "execFileSync"],
  url: ["fileURLToPath", "format", "parse"],
  querystring: ["parse", "stringify", "escape"],
  zlib: ["gzipSync", "inflate"],
  buffer: ["isUtf8", "atob"],
  "fs/promises": ["readFile", "open", "mkdtemp"],
  "timers/promises": ["setTimeout"],
  stream: ["pipeline", "finished"],
};
for (const [id, names] of Object.entries(pick)) {
  const m = require(id);
  console.log(id.padEnd(16), names.map((n) => `${m[n].name}/${m[n].length}`).join(" "));
}
console.log("own keys        ", ["length", "name"].map((k) => Object.getOwnPropertyNames(require("fs").readFileSync).includes(k)).join(","));
console.log("bound           ", require("fs").readFileSync.bind(null, "x").length);
