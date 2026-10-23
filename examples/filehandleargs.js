// `FileHandle` argument checks, `fsPromises.readFile/writeFile/appendFile`
// given a handle (reading and writing from its current position), and
// `readLines` over a handle. Each check is node's, made before any I/O.
//
// The temp directory is scrubbed, including macOS's /private spelling of it.
const fs = require('fs'), path = require('path'), os = require('os'), fsp = require('fs/promises');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fh2-'));
const p = path.join(dir, 'a.txt');
fs.writeFileSync(p, 'abc\ndef\nghi');
const scrub = (s) => String(s).split(`/private${dir}`).join('<dir>').split(dir).join('<dir>');
const err = (e) => console.log('ERR', e.name, e.code, e.syscall, scrub(e.message));
(async () => {
  const fh = await fsp.open(p, 'r+');
  for (const bad of [5, 'str', true, [1], () => 1, { buffer: 'x' }, { buffer: Buffer.alloc(2), offset: -1 }]) {
    try { await fh.read(bad); console.log('ok'); } catch (e) { err(e); }
  }
  try { await fh.read(Buffer.alloc(2), 0, 2, -2); } catch (e) { err(e); }
  try { await fh.read(Buffer.alloc(2), 1.5); } catch (e) { err(e); }
  try { await fh.write(5); } catch (e) { err(e); }
  try { await fh.write(Buffer.alloc(2), 3); } catch (e) { err(e); }
  try { await fh.truncate('x'); } catch (e) { err(e); }
  try { await fh.chmod('zz'); } catch (e) { err(e); }
  try { await fh.stat('x'); } catch (e) { err(e); }
  try { await fh.readv('x'); } catch (e) { err(e); }
  try { await fh.writev([1]); } catch (e) { err(e); }
  console.log(JSON.stringify(await fsp.readFile(fh, 'utf8')));
  console.log(JSON.stringify(await fsp.readFile(fh, 'utf8')));
  await fsp.writeFile(fh, '+w');
  await fsp.appendFile(fh, '+a');
  console.log(JSON.stringify(fs.readFileSync(p, 'utf8')));
  const lines = [];
  const fh2 = await fsp.open(p);
  for await (const l of fh2.readLines()) lines.push(l);
  console.log(lines);
  console.log((await fh.stat()).isFile());
  await fh.close();
  try { await fsp.open(p, 'zz'); } catch (e) { err(e); }
  try { await fsp.open(5); } catch (e) { err(e); }
  try { await fsp.open(dir, 'w'); } catch (e) { err(e); }
  const fh3 = await fsp.open(path.join(dir, 'new.txt'), 'wx', 0o600);
  console.log((fs.statSync(path.join(dir, 'new.txt')).mode & 0o777).toString(8));
  await fh3.close();
  try { await fsp.open(path.join(dir, 'new.txt'), 'wx'); } catch (e) { err(e); }
  console.log(typeof fsp.open, 'open' in fsp, typeof require('fs').promises.open);
  fs.rmSync(dir, { recursive: true });
})().catch((e) => console.log('FATAL', e));
