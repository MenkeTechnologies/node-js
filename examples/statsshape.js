// `fs.Stats` and `BigIntStats`. `{ bigint: true }` was ignored (every field a
// Number, no `*Ns`), a Stats inspected as a bare `{ … }` without its class or
// the four `Symbol(partial*Ns)` remainders node keeps as own properties, the
// `is*` methods could be called but not READ (`typeof stat.isFile` was
// "undefined"), and `fs.Stats` / `fs.Dirent` did not exist.
// Every number is scrubbed: they are this machine's inode, times and sizes.
const fs = require('fs'), util = require('util');
const scrub = (s) => s.replace(/\d+(\.\d+)?n?/g, 'N');
const s = fs.statSync(__filename), b = fs.statSync(__filename, { bigint: true });
console.log(scrub(util.inspect(s)));
console.log(scrub(util.inspect(b)));
console.log(Object.keys(s).join(), '|', Object.keys(b).join());
console.log(Object.getOwnPropertySymbols(s).map(String).join(), Object.getOwnPropertySymbols(b).length);
console.log(s.constructor.name, b.constructor.name, typeof b.atime, b.atime instanceof Date, b.isFile(), typeof s.isFile, typeof b.atimeNs, typeof b.size);
console.log(b.atimeMs === BigInt(b.atime.getTime()), b.atimeNs / 1000000n === b.atimeMs, BigInt(Math.floor(s.atimeMs)) === b.atimeMs);
console.log(Object.getOwnPropertySymbols(s).map((k) => s[k] === Number(b[k.description.replace(/^k?partial/i, '').replace(/^./, (c) => c.toLowerCase())] % 1000000000n)).join());
console.log(JSON.stringify(s).includes('Symbol'), (() => { try { return JSON.stringify(b); } catch (e) { return e.message; } })());
console.log(typeof fs.fstatSync(fs.openSync(__filename), { bigint: true }).size, typeof fs.lstatSync('/', { bigint: true }).ino);
fs.stat('/', { bigint: true }, (e, st) => {
  console.log('cb', typeof st.dev);
  require('fs/promises').stat('/', { bigint: true }).then((st2) => console.log('pr', typeof st2.dev));
});
const d = fs.readdirSync('/', { withFileTypes: true })[0];
console.log(typeof fs.Stats, s instanceof fs.Stats, b instanceof fs.Stats, d instanceof fs.Dirent, Object.getPrototypeOf(s) === fs.Stats.prototype, Object.keys(fs).includes('Stats'));
