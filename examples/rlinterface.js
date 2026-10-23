// readline: with no `output` a question or prompt writes nothing (it fell back
// to the process's stdout), a non-terminal `rl.write(data)` is input — lines
// are emitted, the pending question answered — and the instance carries
// node's `line`/`terminal`/`input`/`output` fields and `closed` once closed.
const readline = require('readline'); const { PassThrough } = require('stream');
const got = [];
const out = new PassThrough(); out.on('data', (d) => got.push('out:' + d));
const rl = readline.createInterface({ input: new PassThrough(), output: out });
rl.on('line', (l) => got.push('line:' + l));
rl.question('Q? ', (a) => got.push('ans:' + a));
rl.write('typed\n');
rl.setPrompt('$ '); rl.prompt();
rl.write('more\n');
const rl2 = readline.createInterface({ input: new PassThrough() });
rl2.on('line', (l) => got.push('line2:' + l)); rl2.write('a\nb'); rl2.question('nothing', () => {}); rl2.prompt();
setImmediate(() => { rl.close(); rl2.close(); console.log(got.join(' | '), rl.closed, rl.terminal, typeof rl.input, rl.output === out); });
