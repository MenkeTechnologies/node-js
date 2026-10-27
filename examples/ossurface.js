// `os`: `userInfo()` reads the password database (uid/gid were -1 and the
// fields came from $USER/$HOME/$SHELL, in another key order; `encoding:
// 'buffer'` was ignored); `getPriority`/`setPriority` validate their arguments
// and raise node's SystemError (ERR_SYSTEM_ERROR, with `info`/`errno`/
// `syscall`) on failure (they silently answered); the value functions carry
// node's `[Symbol.toPrimitive]` (`${os.arch}` is the architecture); a
// module's `constants` is one object (`fs.constants === fs.constants`, shared
// with fs/promises — each read built a new one); `os.constants.signals` is
// frozen.
const os = require("os");
const t = (f) => { try { const v = f(); return typeof v === "object" ? JSON.stringify(v) : String(v); } catch (e) { return e.name + ":" + e.code + ":" + e.message; } };
const u = os.userInfo(), b = os.userInfo({ encoding: "buffer" });
console.log(Object.keys(u).join(), u.uid === process.getuid(), u.gid === process.getgid(), Buffer.isBuffer(b.username), b.username.toString() === u.username, typeof u.homedir, typeof u.shell);
// The inherited nice value is environment state (the macOS CI runner starts
// processes at -10), so pin it first: raising it needs no privilege, and the
// reads below then check that getPriority sees what setPriority set.
os.setPriority(19);
for (const a of [[undefined], [0], [1.5], [2 ** 31], ["x"]]) console.log("get", JSON.stringify(a), t(() => os.getPriority(...a)));
for (const a of [[0, 20], [0, "x"], [-21], [0, 1.5], [999999, 0]]) console.log("set", JSON.stringify(a), t(() => os.setPriority(...a)));
try { os.setPriority(999999, 0); } catch (e) { console.log(e.name, Object.keys(e).join(), JSON.stringify(e.info), e.errno === -os.constants.errno.ESRCH, e.syscall, String(e).split(":")[0], e instanceof Error); }
console.log(String(os.arch) === process.arch, `${os.platform}` === process.platform, os.freemem + 0 > 0, +os.uptime > 0, os.arch[Symbol.toPrimitive].name === "", Symbol.toPrimitive in os.cpus);
console.log(require("fs").constants === require("fs/promises").constants, require("os").constants === require("os").constants, require("crypto").constants === require("crypto").constants, require("constants") === require("constants"));
console.log(Object.isFrozen(os.constants.signals), Object.isFrozen(os.constants), Object.isFrozen(os.constants.errno));
