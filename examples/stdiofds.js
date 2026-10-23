// Descriptors 0–2 are the process's stdin/stdout/stderr for the fd API, as
// in node: `fs.writeSync(1, s)` writes to stdout and `fs.fstatSync(0)` stats
// stdin (both were EBADF — only descriptors node-js opened were known).
// `fsync` follows libuv's sequence (a pipe is EINVAL), and a Stats/Dirent
// answers `isCharacterDevice`/`isFIFO`/`isBlockDevice`/`isSocket` from the
// real file type (all four were always false).
const fs = require('fs');
for (const fd of [0, 1, 2]) { try { const s = fs.fstatSync(fd); console.log(fd, typeof s.mode, s.isFile() || s.isFIFO() || s.isCharacterDevice()); } catch (e) { console.log(fd, e.code); } }
fs.writeSync(1, 'w1\n'); fs.writeSync(2, ''); 
try { fs.fsyncSync(1); console.log('fsync ok'); } catch (e) { console.log('fsync', e.code); }
fs.fstat(1, (e, s) => console.log('cb', e && e.code, typeof (s && s.size)));
const n = fs.statSync('/dev/null'); console.log('null', n.isCharacterDevice(), n.isFIFO(), n.isFile(), n.isBlockDevice(), n.isSocket());
