// child_process: execSync / spawnSync with fixed deterministic commands.
const { execSync, spawnSync } = require("child_process");

console.log(execSync("echo hello world").toString().trim());
console.log(execSync("printf '%s-%s' a b").toString());

const r = spawnSync("printf", ["%s\n", "spawned"]);
console.log(r.stdout.toString().trim());
console.log(r.status);

const cat = spawnSync("cat", [], { input: "piped input\n" });
console.log(cat.stdout.toString().trim());

console.log(execSync("echo one; echo two").toString().trim());

const wc = spawnSync("wc", ["-c"], { input: "12345" });
console.log(wc.stdout.toString().trim());

console.log(execSync("true && echo ok").toString().trim());
