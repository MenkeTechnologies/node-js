// child_process.spawn delivers its child's life as events on a later turn: an
// 'error' then 'close' -2 for a binary that cannot start, output straight
// through under stdio 'inherit' (the stream properties null), 'data' on a
// piped stream (a string after setEncoding), and 'exit' with the signal name
// and exitCode/signalCode set. None of these events fired before.
const cp = require('child_process');
const a = cp.spawn('definitely-missing-cmd', ['a']);
console.log('pid', a.pid, typeof a.stdout);
a.on('error', e => console.log('error', e.code, e.message, e.syscall, e.path, e.spawnargs));
a.on('close', (c, s) => console.log('close', c, s));
const b = cp.spawn('sh', ['-c', 'echo hi; echo e >&2'], { stdio: 'inherit' });
console.log('inherit', b.stdout, b.stderr);
b.on('close', c => {
  console.log('b close', c);
  const d = cp.spawn('sh', ['-c', 'echo piped'], { stdio: ['ignore', 'pipe', 'inherit'] });
  d.stdout.setEncoding('utf8');
  d.stdout.on('data', x => console.log(typeof x, JSON.stringify(x)));
  d.on('exit', () => {
    const k = cp.spawn('sh', ['-c', 'kill -TERM $$']);
    k.on('exit', (c, s) => console.log('exit', c, s, k.exitCode, k.signalCode));
  });
});
