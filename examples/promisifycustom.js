// `fn[util.promisify.custom]` on node's own builtins is the function
// `util.promisify(fn)` returns: `setTimeout`/`setImmediate` give the
// `timers/promises` forms, `fs.exists` resolves its boolean, `child_process`'s
// `exec`/`execFile` resolve `{ stdout, stderr }`, and `stream`'s
// `pipeline`/`finished` are the `stream/promises` ones. The read answered
// `undefined` (so did `in`) while `promisify` knew the forms, and
// `require('timers').setTimeout` was not the global `setTimeout`.
const util = require('util'), tp = require('timers/promises'), fs = require('fs'), timers = require('timers'), cp = require('child_process'), stream = require('stream'), sp = require('stream/promises');
const c = util.promisify.custom;
console.log(setTimeout[c] === tp.setTimeout, setImmediate[c] === tp.setImmediate, timers.setTimeout[c] === tp.setTimeout, timers.setTimeout === setTimeout, timers.clearImmediate === clearImmediate);
console.log(util.promisify(setTimeout) === setTimeout[c], util.promisify(fs.exists) === fs.exists[c], fs.exists[c] === fs.exists[c]);
console.log(fs.exists[c].name, fs.exists[c].length, setTimeout[c].name, cp.exec[c].name, cp.execFile[c].name, cp.exec[c] === util.promisify(cp.exec));
console.log(c in setTimeout, c in fs.exists, c in fs.readFile, typeof setInterval[c], typeof fs.read[c]);
console.log(stream.pipeline[c] === sp.pipeline, stream.finished[c] === sp.finished);
(async () => {
  console.log('exists', await fs.exists[c](__filename), await util.promisify(fs.exists)('/nope'));
  console.log('st', await setTimeout[c](1, 'v'));
  const r = await cp.exec[c]('echo hi');
  console.log('exec', JSON.stringify(r));
  try { await cp.exec[c]('echo out; echo err 1>&2; exit 3'); } catch (e) { console.log('execerr', e.code, JSON.stringify(e.stdout), JSON.stringify(e.stderr)); }
  const f = await util.promisify(cp.execFile)('echo', ['x']);
  console.log('execFile', JSON.stringify(f));
})();
