// StringDecoder: an encoding outside node's set is ERR_UNKNOWN_ENCODING (it was
// stored verbatim), `text(buf, offset)` exists, and the held-back state reads
// through the `lastChar`/`lastNeed`/`lastTotal` prototype getters (absent).
// (A lone UTF-16 surrogate cannot be represented here, so that case is left
// out.)
const { StringDecoder } = require("string_decoder");
const t = (f) => { try { return JSON.stringify(f()); } catch (e) { return e.name + ':' + e.code + ':' + e.message; } };
const run = (enc, chunks) => { const d = new StringDecoder(enc); const out = chunks.map((c) => d.write(Buffer.from(c))); out.push(d.end()); return out; };
const cases = [
  () => run('utf8', [[0xe2], [0x82], [0xac]]),
  () => run('utf8', [[0xf0, 0x9f], [0x98], [0x80, 0x41]]),
  () => run('utf8', [[0xe2, 0x82]]),
  () => run('utf8', [[0xff, 0x41]]),
  () => run('utf16le', [[0x41], [0x00, 0x3d, 0xd8], [0x00, 0xde]]),
  () => run('base64', [[0x61], [0x62], [0x63, 0x64]]),
  () => run('base64url', [[0xfb, 0xff]]),
  () => run('hex', [[0xab], [0xcd]]),
  () => run('latin1', [[0xe9, 0xff]]),
  () => run('ascii', [[0xe9, 0x41]]),
  () => new StringDecoder('UTF-8').encoding,
  () => new StringDecoder('ucs2').encoding,
  () => new StringDecoder('binary').encoding,
  () => new StringDecoder('nope'),
  () => { const d = new StringDecoder(); d.write(Buffer.from([0xe2])); return [d.lastNeed, d.lastTotal, d.lastChar && d.lastChar.length]; },
  () => { const d = new StringDecoder('utf8'); return d.text(Buffer.from('héllo'), 1); },
  () => { const d = new StringDecoder('utf8'); return d.end(Buffer.from([0xe2, 0x82])); },
  () => Object.keys(new StringDecoder()),
];
for (const c of cases) console.log(t(c));

const st=(enc,b)=>{const d=new StringDecoder(enc); if(b) d.write(Buffer.from(b)); return [d.lastNeed,d.lastTotal,d.lastChar.length, [...d.lastChar.subarray(0, d.lastTotal - d.lastNeed)]].join('|')};
for (const [e,b] of [['utf8',null],['utf8',[0xe2]],['utf8',[0xf0,0x9f]],['utf8',[0xe2,0x82,0xac]],['utf16le',[0x41]],['utf16le',[0x3d,0xd8]],['base64',[0x61]],['base64',[0x61,0x62]],['hex',[0xab]],['latin1',[1]]]) console.log(e, JSON.stringify(b), st(e,b));
const d=new StringDecoder('utf8'); console.log(JSON.stringify(d.text(Buffer.from([0x41,0xe2,0x82]),0)), d.lastNeed, JSON.stringify(d.end()));
console.log(Object.getOwnPropertyNames(StringDecoder.prototype), typeof Object.getOwnPropertyDescriptor(StringDecoder.prototype,'lastNeed').get);
for (const e of [undefined, null, '', 'UCS-2', 'Base64', 'nope', 5]) { try { console.log(JSON.stringify(e), new StringDecoder(e).encoding) } catch (x) { console.log(JSON.stringify(e), x.code, x.message) } }
