// `child_process` option handling. Nothing in the corpus reached it, and two
// of the most-used options were ignored outright — silently, so a caller got a
// child that ran somewhere else or saw the wrong variables.
//
// Nothing machine-specific is printed: no real cwd, no inherited environment.
const cp = require("child_process");

// `cwd` was ignored, so the child inherited this process's directory.
const pwd = (opts) => cp.spawnSync("sh", ["-c", "pwd"], opts).stdout.toString().trim().replace(/^\/private/, "");
console.log("cwd      ", pwd({ cwd: "/tmp" }), pwd({ cwd: "/" }));
console.log("exec-cwd ", cp.execSync("pwd", { cwd: "/tmp" }).toString().trim().replace(/^\/private/, ""));

// `env` was ignored too — and it REPLACES the environment rather than adding
// to it, so a variable the parent has is NOT visible to the child unless it is
// passed. Both halves matter: the first line shows the value arriving, the
// second that PATH did not come along with it.
const sh = (script, opts) => cp.spawnSync("sh", ["-c", script], opts).stdout.toString().trim();
console.log("env-set  ", sh('echo "[$FOO]"', { env: { FOO: "x" } }));
console.log("env-repl ", sh('echo "[$PATH]"', { env: { FOO: "x" } }));
console.log("env-empty", sh("env | wc -l", { env: {} }).trim());
// Without the option the child inherits, as before.
console.log("env-inherit", sh('test -n "$PATH" && echo has'));
console.log("both     ", sh("pwd; echo $Z", { cwd: "/tmp", env: { Z: "zed" } }).replace(/^\/private/, "").replace("\n", "|"));
console.log("exec-env ", cp.execSync("echo $Q", { env: { Q: "qq" } }).toString().trim());
console.log("file-env ", cp.execFileSync("sh", ["-c", "echo $W"], { env: { W: "ww" } }).toString().trim());

// A failing `execSync` throws a real Error carrying the whole result. It used
// to throw a bare message, so `e.status` — the standard way to read an exit
// code — was undefined and the code was unrecoverable.
let caught = "no throw";
try {
  cp.execSync("echo out; exit 9", { env: {} });
} catch (e) {
  caught = [e instanceof Error, e.status, e.signal, typeof e.pid, e.stdout.toString().trim()].join("|");
}
console.log("throw    ", caught);
// The encoding option applies to the captured pipes on the error too.
let encoded = "no throw";
try {
  cp.execSync("exit 4", { encoding: "utf8", env: {} });
} catch (e) {
  encoded = [e.status, typeof e.stdout].join(",");
}
console.log("throw-enc", encoded);

// `spawnSync` never throws on a non-zero exit; it reports the status.
console.log("status   ", cp.spawnSync("sh", ["-c", "exit 3"]).status, cp.spawnSync("sh", ["-c", "exit 0"]).status);
console.log("input    ", cp.spawnSync("cat", [], { input: "piped" }).stdout.toString());
