// `process.exitCode`, and the `exit` / `beforeExit` events.
//
// The status a program leaves behind is an observable in its own right, and one
// that prints NOTHING: a harness comparing only stdout, or collapsing the exit
// status to zero-vs-nonzero, cannot see it at all. `run.sh` compares the code
// exactly, so this file measures it.
process.on('beforeExit', (c) => console.log('beforeExit', c));
process.on('exit', (c) => console.log('exit', c));

// A `once` listener is deregistered BEFORE it runs, so a second emit misses it
// and `listeners()` is empty afterwards.
process.once('probe', () => console.log('once fired'));
process.on('probe', () => console.log('on fired'));
process.emit('probe');
process.emit('probe');
console.log('listeners left', process.listeners('probe').length);

// Unset reads back as undefined; a numeric string coerces.
console.log('initial', process.exitCode);
process.exitCode = '0x10';
console.log('after string', process.exitCode);
process.exitCode = 3;
console.log('after number', process.exitCode);

// Nothing here calls process.exit(), so the loop drains, `beforeExit` fires,
// `exit` fires with 3, and the process leaves with status 3.
