// readline over a file stream: 'line' events for \n, \r\n and a lone \r, the
// unterminated last line at end of input, then 'close'; `for await` over the
// same; and the EventEmitter surface of fs read/write streams, which had none.
const fs = require('fs');
const os = require('os');
const path = require('path');
const readline = require('readline');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rl-'));
const file = path.join(dir, 'lines.txt');
fs.writeFileSync(file, 'alpha\nbeta\r\ngamma\rdelta\n\nlast');

function events() {
  return new Promise(resolve => {
    const rl = readline.createInterface({ input: fs.createReadStream(file) });
    const seen = [];
    rl.on('line', l => {
      seen.push(l);
      Promise.resolve().then(() => seen.push('micro:' + l));
    });
    rl.once('close', () => { console.log('events', JSON.stringify(seen)); resolve(); });
  });
}

async function iterate() {
  const rl = readline.createInterface({ input: fs.createReadStream(file, 'utf8'), crlfDelay: Infinity });
  let n = 0;
  for await (const line of rl) console.log(++n, JSON.stringify(line));
  const rl2 = readline.createInterface({ input: fs.createReadStream(file) });
  rl2.on('close', () => console.log('closed after break'));
  for await (const line of rl2) { console.log('first', line); break; }
  console.log('after break');
}

function streams() {
  return new Promise(resolve => {
    const r = fs.createReadStream(file, 'utf8');
    const got = [];
    r.once('open', fd => got.push('open:' + typeof fd));
    r.on('data', d => got.push('data:' + d.length));
    r.on('end', () => got.push('end'));
    r.on('close', () => {
      console.log('read', got.join(' '), r.listenerCount('data'));
      const out = path.join(dir, 'out.txt');
      const w = fs.createWriteStream(out);
      w.on('finish', () => console.log('finish', JSON.stringify(fs.readFileSync(out, 'utf8'))));
      w.on('close', () => { fs.rmSync(dir, { recursive: true }); resolve(); });
      w.write('a');
      w.end('b', () => console.log('end callback'));
      console.log('after end()');
    });
  });
}

events().then(iterate).then(streams).then(() => console.log('done'));
