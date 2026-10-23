// `child_process.exec` returns its ChildProcess, as node's does, and the
// callback is that child's first `close` listener — after `exit`, with the
// collected stdout/stderr. `exec` returned `undefined`, so
// `util.promisify(exec)(cmd).child` was missing. A child, a socket and a
// server are also `instanceof EventEmitter` now.
const cp = require('child_process'), util = require('util');
const log = [];
const c = cp.exec('echo hi; echo e >&2; exit 2', (err, so, se) => { log.push(`cb ${err && err.code} ${JSON.stringify(so)} ${JSON.stringify(se)}`); });
console.log(c.constructor.name, typeof c.pid, typeof c.on, c instanceof require('events'), typeof c.stdout.on);
c.on('exit', (code, sig) => log.push(`exit ${code} ${sig}`));
c.on('close', (code) => { log.push(`close ${code}`); console.log(log.join(' | ')); });
c.stdout.on('data', (d) => log.push('data ' + JSON.stringify(String(d))));
const p = util.promisify(cp.exec)('echo z');
console.log(typeof p.child, p.child && typeof p.child.pid);
p.then((r) => console.log('prom', JSON.stringify(r)));
const c2 = cp.exec('true'); console.log('nocb', typeof c2.pid);
console.log(new (require('net').Socket)() instanceof require('events'), require('net').createServer() instanceof require('events'));
