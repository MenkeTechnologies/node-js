// Buffer's binary read/write surface. Twenty-seven of these methods did not
// exist — every float and double, every 64-bit integer, the whole
// variable-width `IntBE`/`UIntLE` family, and three of the small signed
// writers — so `buf.writeInt8(...)` was "not a function".

// Fixed-width integers, round-tripped at their boundary values in both
// endiannesses. The hex is included so a wrong byte order fails visibly rather
// than cancelling out through the matching read.
const fixed = [
  ["Int8", 1, [-128, -1, 127]],
  ["UInt8", 1, [0, 255]],
  ["Int16BE", 2, [-32768, 32767]], ["Int16LE", 2, [-32768, 32767]],
  ["UInt16BE", 2, [258, 65535]], ["UInt16LE", 2, [258, 65535]],
  ["Int32BE", 4, [-2147483648, 2147483647]], ["Int32LE", 4, [-2147483648, 2147483647]],
  ["UInt32BE", 4, [16909060]], ["UInt32LE", 4, [16909060]],
];
for (const [t, w, vals] of fixed) {
  console.log(t.padEnd(9), vals.map((v) => { const b = Buffer.alloc(w); b["write" + t](v, 0); return b.toString("hex") + "=" + b["read" + t](0); }).join(" "));
}

// IEEE-754. A float loses precision that a double keeps, which is the check
// that these are not quietly sharing one implementation.
for (const t of ["FloatBE", "FloatLE", "DoubleBE", "DoubleLE"]) {
  const w = t.startsWith("Float") ? 4 : 8;
  console.log(t.padEnd(9), [1.5, -2.25, 0.1].map((v) => { const b = Buffer.alloc(w); b["write" + t](v, 0); return b.toString("hex") + "=" + b["read" + t](0); }).join(" "));
}

// 64-bit integers exceed what a double holds exactly, so both sides are
// BigInts. These are the extremes of each range.
for (const t of ["BigInt64BE", "BigInt64LE"]) {
  console.log(t.padEnd(12), [-9223372036854775808n, -1n, 9223372036854775807n].map((v) => { const b = Buffer.alloc(8); b["write" + t](v, 0); return b.toString("hex") + "=" + b["read" + t](0); }).join(" "));
}
for (const t of ["BigUInt64BE", "BigUInt64LE"]) {
  console.log(t.padEnd(12), [0n, 18446744073709551615n].map((v) => { const b = Buffer.alloc(8); b["write" + t](v, 0); return b.toString("hex") + "=" + b["read" + t](0); }).join(" "));
}

// The variable-width family takes `byteLength` as an argument, which is why it
// cannot share the fixed-width paths. Signed reads sign-extend from the top bit
// of the LAST byte, not from bit 63.
for (const t of ["IntBE", "IntLE", "UIntBE", "UIntLE"]) {
  const out = [];
  for (let w = 1; w <= 6; w++) {
    const v = t.startsWith("U") ? Math.pow(2, 8 * w) - 1 : -Math.pow(2, 8 * w - 1);
    const b = Buffer.alloc(6);
    b["write" + t](v, 0, w);
    out.push(w + ":" + b.subarray(0, w).toString("hex") + "=" + b["read" + t](0, w));
  }
  console.log(t.padEnd(8), out.join(" "));
}

// Reads at an offset, and the same bytes read both ways.
const src = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]);
console.log("offsets ", src.readUInt16BE(0), src.readUInt16LE(0), src.readUInt32BE(2), src.readUInt32LE(2));
console.log("varoff  ", src.readIntBE(1, 3), src.readUIntLE(1, 3), src.readBigUInt64BE(0));
// Each write returns the offset just past what it wrote, so they chain.
const chain = Buffer.alloc(6);
let at = chain.writeUInt8(1, 0);
at = chain.writeUInt16BE(0x0203, at);
at = chain.writeInt8(-1, at);
console.log("chained ", at, chain.toString("hex"));

// A write that would run past the end is a RangeError. It used to be skipped
// silently while still returning the advanced offset, so the caller was told it
// had succeeded and the bytes were simply lost.
const small = Buffer.alloc(4);
const attempt = (f) => { try { return String(f()); } catch (e) { return e.constructor.name; } };
console.log("bounds  ", attempt(() => small.writeUInt32BE(1, 2)), attempt(() => small.readUInt32BE(1)), attempt(() => small.readBigUInt64BE(0)));
console.log("inrange ", attempt(() => { small.writeUInt32BE(1, 0); return small.toString("hex"); }));

// BINARY input across the stdlib. Only a `Buffer` argument was read as bytes;
// a TypedArray or DataView was stringified first, so `writeFileSync`,
// `gzipSync` and `hash.update` all wrote the text "[object Object]". A Buffer
// only appeared to work because stringifying one yields its utf8 text — wrong
// the moment the bytes are not valid utf8, which is when binary I/O matters.
const fs = require("fs"), zlib = require("zlib"), crypto = require("crypto");
const os = require("os"), path = require("path");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nodejs-bin-"));
const bytes = [0xff, 0xfe, 0x00, 0x41, 0x80];
const buf = Buffer.from(bytes), ta = new Uint8Array(bytes);
const ab = new ArrayBuffer(5);
new Uint8Array(ab).set(bytes);
const dv = new DataView(ab);
const wrote = (name, data) => {
  const p = path.join(dir, name);
  fs.writeFileSync(p, data);
  return fs.readFileSync(p).toString("hex");
};
console.log("fs      ", wrote("a", buf), wrote("b", ta), wrote("c", dv));
console.log("fs-win  ", wrote("d", new Uint8Array(ab, 1, 3)));
console.log("zlib    ", zlib.gunzipSync(zlib.gzipSync(buf)).toString("hex"),
  zlib.gunzipSync(zlib.gzipSync(ta)).toString("hex"),
  zlib.inflateSync(zlib.deflateSync(dv)).toString("hex"));
const h = (d) => crypto.createHash("sha256").update(d).digest("hex");
console.log("hash    ", h(buf) === h(ta), h(buf) === h(dv), h(buf).slice(0, 16));
console.log("hmac    ", crypto.createHmac("sha256", buf).update(ta).digest("hex").slice(0, 16));
// A window into a buffer contributes only its own bytes.
console.log("window  ", h(new DataView(ab, 1, 3)) === h(Buffer.from([0xfe, 0x00, 0x41])));
// Mixed chunk forms accumulate in order.
console.log("chunks  ", crypto.createHash("sha256").update("a").update(new Uint8Array([98]))
  .update(Buffer.from("c")).digest("hex") === h(Buffer.from("abc")));

// `Buffer.from` took anything at all and stringified what it did not recognise:
// `Buffer.from(5)` produced the digit "5" and `Buffer.from(null)` the four
// bytes of "null". Node accepts a string, a byte source, an Array or an
// ARRAY-LIKE object, and rejects the rest.
const tryFrom = (f) => { try { return f(); } catch (e) { return e.constructor.name + "/" + e.code; } };
console.log("from-ok ", [...Buffer.from([1, 2, 3])].join(","), Buffer.from("ab").toString(),
  [...Buffer.from(ta)].length, Buffer.from(new DataView(new ArrayBuffer(3))).length);
console.log("from-alk", [...Buffer.from({ length: 2 })].join(","),
  [...Buffer.from({ length: 2, 0: 65, 1: 66 })].join(","));
console.log("from-bad", tryFrom(() => Buffer.from(5)), tryFrom(() => Buffer.from(null)),
  tryFrom(() => Buffer.from({})), tryFrom(() => Buffer.from(true)));
fs.rmSync(dir, { recursive: true, force: true });
