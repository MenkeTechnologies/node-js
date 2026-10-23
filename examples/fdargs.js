// The descriptor calls check their arguments as node does, before any I/O,
// and report the OS error by its own code. They used to read every argument
// with an infallible ToNumber: a string mode '644' set the permissions
// 0o1204, `writeSync(fd, {})` wrote the 15 bytes of "[object Object]", an
// unknown flag string opened the file read-only, a URL path was opened as
// the file "[object Object]", and every errno Rust's ErrorKind does not
// single out (EBADF, ELOOP, ENAMETOOLONG, ...) was reported as EIO.
//
// Paths are scrubbed (macOS also spells the temp dir with a /private prefix)
// and errno is compared with os.constants rather than printed, since the
// numbers differ between platforms.
const fs = require("fs"), path = require("path"), os = require("os");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "njfd-"));
const p = path.join(dir, "a");
fs.writeFileSync(p, "abcdefgh");
const scrub = (s) => String(s).split(`/private${dir}`).join("<dir>").split(dir).join("<dir>");
const show = (label, f) => {
  try {
    const r = f();
    console.log(label, "ok", typeof r === "number" || r === undefined ? r : typeof r);
  } catch (e) {
    const errnoOk = e.errno === undefined || e.errno === -os.constants.errno[e.code];
    console.log(label, e.name, e.code, e.syscall, errnoOk, scrub(e.message));
  }
};

const fd = fs.openSync(p, "r+");

// A positioned write is pwrite: the descriptor's offset does not move.
fs.writeSync(fd, "XY", 4);
fs.writeSync(fd, Buffer.from("Q"), 0, 1, 6);
fs.writeSync(fd, "Z");
fs.writeSync(fd, Buffer.from("W"), 0, 1);
fs.writevSync(fd, [Buffer.from("V")], 7);
fs.writevSync(fd, [Buffer.from("U")]);
console.log("pwrite  ", fs.readFileSync(p, "utf8"));

// The descriptor itself: validateInt32(fd, 'fd', 0).
show("fd str  ", () => fs.fstatSync("x"));
show("fd frac ", () => fs.fstatSync(1.5));
show("fd neg  ", () => fs.fsyncSync(-1));
show("fd big  ", () => fs.closeSync(2 ** 31));
show("fd null ", () => fs.closeSync(null));
show("fd cb   ", () => fs.close("x", () => {}));
show("rs fd   ", () => fs.readSync("x", Buffer.alloc(1)));
show("rs len0 ", () => fs.readSync("x", Buffer.alloc(1), 0, 0));
show("rs both ", () => fs.readSync("x", 5));
show("read cb ", () => fs.read(-1, Buffer.alloc(1), 0, 1, null, () => {}));

// writeSync's data and window.
show("wr num  ", () => fs.writeSync(fd, 5));
show("wr obj  ", () => fs.writeSync(fd, {}));
show("wr off  ", () => fs.writeSync(fd, Buffer.alloc(2), 3));
show("wr len  ", () => fs.writeSync(fd, Buffer.alloc(2), 0, 3));
show("wr neg  ", () => fs.writeSync(fd, Buffer.alloc(2), -1));
show("writev  ", () => fs.writevSync(fd, [1]));
show("readv   ", () => fs.readvSync(fd, "x"));

// parseFileMode: an octal STRING or a uint32.
fs.fchmodSync(fd, "640");
console.log("fchmod  ", (fs.fstatSync(fd).mode & 0o777).toString(8));
fs.chmodSync(p, "600");
console.log("chmod   ", (fs.statSync(p).mode & 0o777).toString(8));
show("mode zz ", () => fs.fchmodSync(fd, "zz"));
show("mode 8  ", () => fs.chmodSync(p, "8"));
show("mode neg", () => fs.fchmodSync(fd, -1));
show("mode frc", () => fs.fchmodSync(fd, 1.5));
show("mode und", () => fs.fchmodSync(fd));

// The other numeric arguments.
show("len str ", () => fs.ftruncateSync(fd, "x"));
show("len frac", () => fs.ftruncateSync(fd, 1.5));
show("uid str ", () => fs.fchownSync(fd, "a", 1));
show("uid low ", () => fs.fchownSync(fd, -2, 1));
show("time str", () => fs.futimesSync(fd, "x", 1));
show("time nan", () => fs.futimesSync(fd, NaN, 1));
fs.futimesSync(fd, "1000000000", new Date(2e12));
console.log("futimes ", fs.fstatSync(fd).atimeMs, fs.fstatSync(fd).mtimeMs);
fs.utimesSync(p, new Date(3e12), 4e9);
console.log("utimes  ", fs.statSync(p).atimeMs, fs.statSync(p).mtimeMs);
fs.closeSync(fd);

// stringToFlags, and a created file's mode.
show("flag zz ", () => fs.openSync(p, "zz"));
show("flag obj", () => fs.openSync(p, {}));
show("flag frc", () => fs.openSync(p, 1.5));
const made = path.join(dir, "made");
fs.closeSync(fs.openSync(made, "wx", 0o640));
console.log("mode    ", (fs.statSync(made).mode & 0o777) === (0o640 & ~process.umask()));
show("wx again", () => fs.openSync(made, "wx"));
fs.closeSync(fs.openSync(made, fs.constants.O_RDONLY));
const syncFd = fs.openSync(made, "as+");
fs.writeSync(syncFd, "s");
fs.closeSync(syncFd);
console.log("as+     ", fs.readFileSync(made, "utf8"));

// Errors keep their own codes.
fs.symlinkSync(path.join(dir, "l2"), path.join(dir, "l1"));
fs.symlinkSync(path.join(dir, "l1"), path.join(dir, "l2"));
show("eloop   ", () => fs.readFileSync(path.join(dir, "l1")));
show("toolong ", () => fs.statSync(path.join(dir, "x".repeat(300))));
show("notdir  ", () => fs.mkdirSync(path.join(p, "sub")));
const ro = fs.openSync(p, "r");
show("ebadf w ", () => fs.writeSync(ro, "x"));
show("ebadf wv", () => fs.writevSync(ro, [Buffer.from("x")]));
fs.closeSync(ro);
const wo = fs.openSync(p, "a");
show("ebadf rv", () => fs.readvSync(wo, [Buffer.alloc(1)]));
fs.closeSync(wo);

// getValidatedPath: a string, a Uint8Array or a file: URL, without NUL.
show("path num", () => fs.statSync(5));
show("path obj", () => fs.statSync({}));
show("path und", () => fs.openSync(undefined));
show("path nul", () => fs.statSync("a\0b"));
show("path u16", () => fs.statSync(new Uint16Array(2)));
show("src     ", () => fs.copyFileSync(1, p));
show("dest    ", () => fs.renameSync(p, true));
show("url     ", () => fs.statSync(new URL("file://" + p)).size);
show("url http", () => fs.statSync(new URL("http://x/")));
// A POSIX file URL names no host and encodes no `/` (the host message names
// the platform, so only the code is printed).
for (const u of ["file://host/x", "file:///a%2Fb", "file:///a%2fb", "file://localhost/"]) {
  try {
    console.log("url chk ", u, typeof fs.statSync(new URL(u)));
  } catch (e) {
    console.log("url chk ", u, e.code);
  }
}
show("buffer  ", () => fs.statSync(Buffer.from(p)).size);
show("u8      ", () => fs.readFileSync(new TextEncoder().encode(p)).length);
console.log("exists  ", fs.existsSync(new URL("file://" + p)), fs.existsSync({}));

// A read stream over a caller's descriptor.
const sfd = fs.openSync(p, "r");
const s = fs.createReadStream(null, { fd: sfd, encoding: "utf8" });
const events = [];
for (const e of ["open", "ready", "data", "end", "close"]) s.on(e, () => events.push(e));
s.on("close", () => {
  let open = true;
  try {
    fs.fstatSync(sfd);
  } catch {
    open = false;
  }
  console.log("stream  ", events.join(","), open);
  fs.rmSync(dir, { recursive: true });
});
