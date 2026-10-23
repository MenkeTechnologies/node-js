// Write streams over a descriptor (`{ fd }`, `autoClose`) and over a
// FileHandle (`fh.createWriteStream()`). The `fd` option was ignored — the
// stream opened the path argument, so `createWriteStream(null, { fd })`
// wrote to a file named "null" — and `autoClose: false` still closed the
// descriptor and emitted `close`.
const fs = require('fs'), path = require('path'), os = require('os'), fsp = require('fs/promises');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-'));
const p = path.join(dir, 'w.txt');
(async () => {
  const fh = await fsp.open(p, 'w');
  let closed = 0; fh.on('close', () => closed++);
  const ws = fh.createWriteStream();
  const ev = [];
  for (const e of ['open', 'ready', 'finish', 'close']) ws.on(e, () => ev.push(e));
  ws.write('ab');
  ws.end('cd');
  await new Promise((r) => ws.on('finish', r));
  await new Promise((r) => setTimeout(r, 20));
  console.log(fs.readFileSync(p, 'utf8'), ev.join(','), fh.fd, closed);
  const fd = fs.openSync(p, 'a');
  const ws2 = fs.createWriteStream(null, { fd, autoClose: false });
  const ev2 = [];
  for (const e of ['open', 'ready', 'finish', 'close']) ws2.on(e, () => ev2.push(e));
  ws2.end('!');
  await new Promise((r) => setTimeout(r, 20));
  fs.writeSync(fd, '?');
  fs.closeSync(fd);
  console.log(fs.readFileSync(p, 'utf8'), ev2.join(','));
  fs.rmSync(dir, { recursive: true });
})();
