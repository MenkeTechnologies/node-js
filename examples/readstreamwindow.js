// `fs.createReadStream` honours `start`/`end` (inclusive) and
// `highWaterMark`, validates them synchronously, keeps `bytesRead`, and
// decodes each chunk through a StringDecoder so a character split between two
// chunks arrives whole. It read the whole file as one chunk whatever the
// options, and `bytesRead` was undefined.
const fs = require('fs'), path = require('path'), os = require('os');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rs-'));
const p = path.join(dir, 'f');
fs.writeFileSync(p, 'abcdefghijklmnopqrstuvwxyz');
const run = (label, opts) => new Promise((res) => {
  const chunks = [];
  const s = fs.createReadStream(p, opts);
  s.on('data', (c) => chunks.push(typeof c === 'string' ? c : c.toString()));
  s.on('error', (e) => { console.log(label, 'error', e.code, e.message); res(); });
  s.on('end', () => { console.log(label, JSON.stringify(chunks), s.bytesRead); res(); });
});
(async () => {
  await run('all  ', {});
  await run('start', { start: 3 });
  await run('end  ', { end: 4 });
  await run('both ', { start: 2, end: 5 });
  await run('hwm  ', { highWaterMark: 10 });
  await run('hwm+e', { highWaterMark: 4, start: 1, end: 9, encoding: 'utf8' });
  await run('s>e  ', { start: 5, end: 2 }).catch(() => {});
  try { fs.createReadStream(p, { start: -1 }); } catch (e) { console.log('neg  ', e.code, e.message); }
  try { fs.createReadStream(p, { start: 'x' }); } catch (e) { console.log('str  ', e.code, e.message); }
  try { fs.createReadStream(p, { start: 5, end: 2 }); } catch (e) { console.log('s>e  ', e.code, e.message); }
  await run('past ', { start: 100 });
  fs.writeFileSync(p, 'é'.repeat(5));
  await run('utf8 ', { highWaterMark: 3, encoding: 'utf8' });
  const fd = fs.openSync(p, 'r');
  await run('fdpos', { fd, start: 2, highWaterMark: 4 });
  fs.writeFileSync(p, Buffer.from([0xe2, 0x82, 0xac, 0x41, 0xf0, 0x9f, 0x98, 0x80]));
  const parts = [];
  await new Promise((r) => {
    const s = fs.createReadStream(p, { highWaterMark: 2, encoding: "base64" });
    s.on("data", (c) => parts.push(c));
    s.on("end", r);
  });
  console.log("b64  ", JSON.stringify(parts), parts.join("") === fs.readFileSync(p).toString("base64"));
  const out = path.join(dir, "o");
  await new Promise((r) => {
    const w = fs.createWriteStream(out);
    fs.createReadStream(p, { highWaterMark: 3, start: 1 }).pipe(w);
    w.on("finish", r);
  });
  console.log("pipe ", fs.readFileSync(out).toString("hex"));
  fs.rmSync(dir, { recursive: true });
})();
