// What an `fs` failure reports. `err.code`, `err.syscall` and the message text
// are all branched on by packages — `if (err.code === 'ENOENT')` is the most
// common error check in node code — and the syscall half named the JS FUNCTION
// where node names the C call: `statSync` instead of `stat`, `unlinkSync`
// instead of `unlink`.
//
// Paths are replaced before printing, since a temp directory name is different
// on every run.
const fs = require("fs"), path = require("path"), os = require("os");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "njfs-"));
const file = path.join(dir, "x");
fs.writeFileSync(file, "hi");
const gone = path.join(dir, "nope");
// macOS resolves `/var` to `/private/var`, so a path node has canonicalized
// carries a `/private` prefix that Linux never produces. Both are scrubbed, or
// the frozen transcript would be the one taken on whichever platform blessed
// it — the same mistake as pinning a timezone.
const scrub = (s) => String(s).split(`/private${dir}`).join("<dir>").split(dir).join("<dir>");
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, JSON.stringify([e.code, e.syscall, scrub(e.path), scrub(e.message)])); }
};

// The syscall is the C call, not the JS name.
show("stat         ", () => fs.statSync(gone));
show("lstat        ", () => fs.lstatSync(gone));
show("unlink       ", () => fs.unlinkSync(gone));
show("rmdir        ", () => fs.rmdirSync(gone));
show("chmod        ", () => fs.chmodSync(gone, 0o644));
show("access       ", () => fs.accessSync(gone));
// …and for the ones whose spelling differs, or that fail inside another call.
show("readdir      ", () => fs.readdirSync(gone));
show("readFile     ", () => fs.readFileSync(gone));
show("open         ", () => fs.openSync(gone, "r"));
show("truncate     ", () => fs.truncateSync(gone, 0));
show("copyFile     ", () => fs.copyFileSync(gone, file));
show("mkdir-eexist ", () => fs.mkdirSync(dir));
// `realpath` fails inside the `lstat` it walks the path with, and reports the
// path it had already resolved.
show("realpath     ", () => fs.realpathSync(gone));
// A two-argument call names BOTH paths in the message, and `err.path` is the
// first one — parsing to the end of the line swallowed the arrow into it.
show("rename       ", () => fs.renameSync(gone, file));
// A DIRECTORY opens fine and fails at the read, which carries no path.
show("read-dir     ", () => fs.readFileSync(dir));
show("write-dir    ", () => fs.writeFileSync(dir, "x"));
// `readlink` on a regular file is EINVAL, not ENOENT.
show("readlink     ", () => fs.readlinkSync(file));
// A `Stats` object's own enumerable keys are the fourteen numeric fields, in
// node's order: the four Date fields are accessors on the prototype there, so
// a spread or a `JSON.stringify` must not carry them.
show("stat-keys    ", () => Object.keys(fs.statSync(file)));
show("stat-spread  ", () => [Object.keys({ ...fs.statSync(file) }).length, Object.keys(JSON.parse(JSON.stringify(fs.statSync(file)))).length]);
show("stat-dates   ", () => { const s = fs.statSync(file); return [s.atime instanceof Date, s.mtime instanceof Date, s.ctime instanceof Date, s.birthtime instanceof Date]; });
show("stat-methods ", () => { const s = fs.statSync(file); return [s.isFile(), s.isDirectory(), s.isSymbolicLink(), s.size]; });
show("stat-dir     ", () => { const s = fs.statSync(dir); return [s.isFile(), s.isDirectory()]; });
// …and the calls that SUCCEED still do.
show("roundtrip    ", () => { const p = path.join(dir, "y"); fs.writeFileSync(p, "abc"); fs.appendFileSync(p, "!"); const out = fs.readFileSync(p, "utf8"); fs.unlinkSync(p); return [out, fs.existsSync(p)]; });
fs.rmSync(dir, { recursive: true, force: true });
