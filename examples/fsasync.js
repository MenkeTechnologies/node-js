// An error-first callback is handed an ERROR OBJECT. Every `fs` callback form
// was handed the message STRING instead, so `err.code` — the most common error
// check in node code — was `undefined`, and so were `err.syscall`, `err.path`
// and `err.message`. Printing the error said `undefined`. The promise forms
// already built the object, so the two halves of the same API disagreed.
const fs = require("fs"), fsp = require("fs/promises"), path = require("path"), os = require("os");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "njfa-"));
const file = path.join(dir, "x");
fs.writeFileSync(file, "hi");
const gone = path.join(dir, "nope");
// Both the plain and the canonical spelling of the temp path: macOS resolves
// `/var` to `/private/var` and Linux does not.
const scrub = (s) => String(s).split(`/private${dir}`).join("<dir>").split(dir).join("<dir>");
const report = (label, e, extra) =>
  console.log(label, JSON.stringify([e === null, e && e.code, e && e.syscall, e instanceof Error, e && scrub(e.message), extra]));
const call = (label, fn, pick) =>
  new Promise((r) => fn((e, a) => { report(label, e, pick ? pick(a) : undefined); r(); }));

(async () => {
  // A FAILING callback gets a real Error with node's fields.
  await call("stat       ", (k) => fs.stat(gone, k));
  await call("access     ", (k) => fs.access(gone, k));
  await call("unlink     ", (k) => fs.unlink(gone, k));
  await call("rmdir      ", (k) => fs.rmdir(gone, k));
  await call("readdir    ", (k) => fs.readdir(gone, k));
  await call("readFile   ", (k) => fs.readFile(gone, k));
  await call("open       ", (k) => fs.open(gone, "r", k));
  await call("mkdir      ", (k) => fs.mkdir(dir, k));
  await call("rename     ", (k) => fs.rename(gone, file, k));
  await call("copyFile   ", (k) => fs.copyFile(gone, file, k));
  // …including the directory read, which names `read` and carries no path.
  await call("read-dir   ", (k) => fs.readFile(dir, k));
  // A SUCCEEDING callback gets `null` first, then its result — the encoding
  // still decides the representation.
  await call("ok-stat    ", (k) => fs.stat(file, k), (s) => [s.size, s.isFile()]);
  await call("ok-readFile", (k) => fs.readFile(file, "utf8", k), (s) => s);
  await call("ok-hex     ", (k) => fs.readFile(file, "hex", k), (s) => s);
  await call("ok-readdir ", (k) => fs.readdir(dir, k), (a) => a.sort());
  await call("ok-write   ", (k) => fs.writeFile(path.join(dir, "y"), "z", k), () => fs.readFileSync(path.join(dir, "y"), "utf8"));
  // The promise forms reject with the same object.
  for (const [label, fn] of [
    ["p-stat     ", () => fsp.stat(gone)],
    ["p-readFile ", () => fsp.readFile(gone)],
    ["p-rename   ", () => fsp.rename(gone, file)],
  ]) {
    try { await fn(); console.log(label, "no throw"); }
    catch (e) { report(label, e); }
  }
  console.log("p-ok       ", JSON.stringify(await fsp.readFile(file, "utf8")));
  fs.rmSync(dir, { recursive: true, force: true });
})();
