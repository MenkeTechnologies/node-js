// `fs/promises.open` and `FileHandle`, a port of node's
// `lib/internal/fs/promises.js`. `open` did not exist, so every
// FileHandle-based API (`fh.read`, `fh.readFile`, `fh.readLines`, passing a
// handle to `fsPromises.readFile`) was unreachable.
//
// The temp directory is scrubbed, including macOS's /private spelling of it.
const fs = require('fs'), path = require('path'), os = require('os');
const fsp = require('fs/promises');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fh-'));
const p = path.join(dir, 'a.txt');
fs.writeFileSync(p, 'hello world\nline2\n');
const scrub = (s) => String(s).split(`/private${dir}`).join('<dir>').split(dir).join('<dir>');
const err = (e) => console.log('ERR', e.name, e.code, e.syscall, scrub(e.message));
(async () => {
  const fh = await fsp.open(p, 'r+');
  console.log(typeof fh.fd, fh.constructor.name, fh instanceof require('events'), typeof fh.close, Object.keys(fh).includes('close'));
  const b = Buffer.alloc(5);
  let r = await fh.read(b, 0, 5, 0);
  console.log(r.bytesRead, r.buffer === b, b.toString(), Object.getPrototypeOf(r), Object.keys(r));
  r = await fh.read({ buffer: Buffer.alloc(3), position: 6 });
  console.log(r.bytesRead, r.buffer.toString());
  r = await fh.read(Buffer.alloc(4), { position: 1 });
  console.log(r.bytesRead, r.buffer.toString());
  r = await fh.read();
  console.log(r.bytesRead, r.buffer.length, JSON.stringify(r.buffer.subarray(0, r.bytesRead).toString()));
  r = await fh.read(Buffer.alloc(4), 0, 0, 0);
  console.log(r.bytesRead);
  console.log(JSON.stringify(await fh.readFile('utf8')));
  const st = await fh.stat();
  console.log(st.size, st.isFile());
  let w = await fh.write('XY', 0);
  console.log(w.bytesWritten, w.buffer);
  w = await fh.write(Buffer.from('abc'), 0, 2, 3);
  console.log(w.bytesWritten, fs.readFileSync(p, 'utf8').slice(0, 8));
  w = await fh.write(Buffer.from('QRS'), { offset: 1, position: 9 });
  console.log(w.bytesWritten, fs.readFileSync(p, 'utf8').slice(0, 12));
  await fh.truncate(5);
  console.log(fs.readFileSync(p, 'utf8'));
  await fh.writeFile('new');
  console.log(JSON.stringify(fs.readFileSync(p, 'utf8')));
  await fh.appendFile('+more');
  console.log(JSON.stringify(fs.readFileSync(p, 'utf8')));
  const vr = await fh.readv([Buffer.alloc(2), Buffer.alloc(3)], 0);
  console.log(vr.bytesRead, vr.buffers.map(String));
  const vw = await fh.writev([Buffer.from('1'), Buffer.from('2')], 0);
  console.log(vw.bytesWritten, JSON.stringify(fs.readFileSync(p, 'utf8')));
  await fh.sync(); await fh.datasync(); await fh.chmod(0o644);
  console.log((fs.statSync(p).mode & 0o777).toString(8));
  let closed = 0; fh.on('close', () => closed++);
  const c1 = fh.close(); const c2 = fh.close();
  console.log(c1 === c2, fh.fd, closed);
  await c1;
  await fh.close();
  try { await fh.read(Buffer.alloc(1)); } catch (e) { err(e); }
  try { await fh.stat(); } catch (e) { err(e); }
  try { await fh.writeFile('x'); } catch (e) { err(e); }
  try { await fh.appendFile('x'); } catch (e) { err(e); }
  try { await fh.truncate(); } catch (e) { err(e); }
  try { await fsp.open(path.join(dir, 'nope')); } catch (e) { err(e); }
  const fh2 = await fsp.open(p);
  try { await fh2.read(Buffer.alloc(2), 5, 1); } catch (e) { err(e); }
  try { await fh2.read(Buffer.alloc(2), 0, 3); } catch (e) { err(e); }
  try { await fh2.read(5); } catch (e) { err(e); }
  try { await fh2.read({ buffer: 5 }); } catch (e) { err(e); }
  try { await fh2.write(Buffer.from('z')); } catch (e) { err(e); }
  // close while a read is in flight
  const pr = fh2.read(Buffer.alloc(2), 0, 2, 0);
  const pc = fh2.close();
  console.log(fh2.fd !== -1);
  console.log((await pr).bytesRead); await pc; console.log(fh2.fd);
  const fh3 = await fsp.open(p, 'w');
  await fh3.writeFile(Buffer.from([0xff, 0x00, 0x41]));
  await fh3.close();
  console.log(fs.readFileSync(p));
  {
    const fh4 = await fsp.open(p);
    console.log((await fh4.readFile()).length);
    await fh4[Symbol.asyncDispose]();
    console.log(fh4.fd);
  }
  fs.rmSync(dir, { recursive: true });
})().catch((e) => console.log('FATAL', e));
