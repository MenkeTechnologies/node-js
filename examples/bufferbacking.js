// Every Buffer is a view onto an ArrayBuffer. `buf.buffer` threw on every
// Buffer node-js made — a Buffer owned a private byte array with no buffer
// behind it — and a Buffer that DID share an ArrayBuffer (`Buffer.from(ab, off,
// len)`) had every reader but indexing see the whole store instead of its
// window. Pool-dependent facts (a small Buffer's byteOffset, its buffer's
// size) are deliberately not printed: node allocates those from a shared pool.

// `.buffer` exists and aliases the Buffer's bytes in both directions.
const a = Buffer.alloc(4, 0x61);
console.log(a.buffer instanceof ArrayBuffer, a.buffer.byteLength, a.byteOffset);
new Uint8Array(a.buffer)[1] = 0x7a;
a[2] = 0x79;
console.log(a.toString(), [...new Uint8Array(a.buffer)]);
const s = Buffer.from("hello");
console.log(Buffer.from(s.buffer, s.byteOffset, s.length).toString());
console.log(new DataView(s.buffer, s.byteOffset, s.length).getUint8(1));
const f = Buffer.from([0, 0, 128, 63]);
console.log(new Float32Array(f.buffer, f.byteOffset, 1)[0]);
console.log(require("zlib").gzipSync("x").buffer instanceof ArrayBuffer);

// A window onto part of an ArrayBuffer: every reader sees the window only.
const ab = new ArrayBuffer(10);
new Uint8Array(ab).set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
const w = Buffer.from(ab, 3, 4);
console.log(w, [...w], w.map((x) => x * 2), Array.from(w), w.toJSON());
console.log(Object.keys(w), w.hasOwnProperty(3), w.hasOwnProperty(4), 4 in w);
console.log(w.readUInt32BE(0), w.equals(Buffer.from([3, 4, 5, 6])), `${w}`.length);
console.log(String(Buffer.from(ab, 7, 2)), JSON.stringify(Buffer.from(ab, 8)));
console.log(require("crypto").createHash("md5").update(w).digest("hex"));
console.log(Object.getOwnPropertyDescriptor(w, 0).value, Object.getOwnPropertyDescriptor(w, 3).value);
const r = new ArrayBuffer(4);
crypto.getRandomValues(Buffer.from(r, 1, 2));
console.log(r.byteLength, new Uint8Array(r)[0], new Uint8Array(r)[3]);
w.reverse();
console.log([...new Uint8Array(ab)]);
w.fill(9);
console.log([...new Uint8Array(ab)]);
w.writeUInt16LE(0xffee, 1);
console.log([...new Uint8Array(ab)], w.buffer === ab, w.byteOffset);

// `subarray` and the deprecated `slice` are windows onto the same memory.
const p = Buffer.alloc(6, 0x61);
const sub = p.subarray(1, 3);
sub.fill("z");
p.slice(4)[0] = 0x51;
console.log(p.toString(), sub.buffer === p.buffer, sub.byteOffset - p.byteOffset, sub.length);
console.log(Buffer.from(ab, 2, 6).subarray(1, 3), Buffer.from(ab, 2, 6).subarray(-2).byteOffset);

// Detaching the backing store empties every Buffer over it.
const d = Buffer.alloc(3, 1);
d.buffer.transfer();
console.log(d.length, d.byteLength, d[0], Object.keys(d), d.buffer.detached);
try {
  [...d];
} catch (e) {
  console.log(e.constructor.name, e.message);
}
