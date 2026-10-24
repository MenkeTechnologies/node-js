// Small Buffers are slices of one shared pool (lib/buffer.js `allocPool`):
// `Buffer.from` of a string or an array-like, `allocUnsafe`, `concat` and
// `copyBytesFrom` advance an 8-byte-aligned cursor through a 64 KiB pool,
// while `alloc`, `allocUnsafeSlow` and `of` own their memory. The pool is
// untransferable. Every Buffer used to own an exactly-sized buffer at offset 0.
//
// Results are gathered first and printed once: in node a write to a FILE or a
// terminal stdout also takes pool space, so nothing is printed between the
// allocations being measured.
const out = [];
const log = (...v) => out.push(v.join(" "));

const a = Buffer.from("abc");
log("first", a.byteOffset, a.buffer.byteLength);
const b = Buffer.from("defgh");
log("second", b.byteOffset, b.buffer === a.buffer);
log("allocUnsafe", Buffer.allocUnsafe(10).byteOffset, "array", Buffer.from([1, 2, 3]).byteOffset);
log("concat", Buffer.concat([a, b]).byteOffset, "typed", Buffer.from(new Uint8Array(3)).byteOffset);
log("arraylike", Buffer.from({ length: 2 }).byteOffset, "copy", Buffer.from(a).byteOffset);
log("json", Buffer.from({ type: "Buffer", data: [1, 2] }).byteOffset, "copyBytesFrom", Buffer.copyBytesFrom(new Uint16Array([1, 2])).byteOffset);
log("hex-empty", Buffer.from("zz", "hex").byteOffset, Buffer.from("zz", "hex").length, "next", Buffer.from("x").byteOffset);

// Never pooled.
log("alloc", Buffer.alloc(3).byteOffset, Buffer.alloc(3).buffer.byteLength);
log("slow", Buffer.allocUnsafeSlow(3).buffer.byteLength, "of", Buffer.of(1, 2).buffer.byteLength);
log("empty", Buffer.from("").buffer.byteLength, Buffer.from([]).buffer.byteLength, Buffer.allocUnsafe(0).buffer.byteLength);

// The size limit is half the pool, by an estimate of 4 bytes per UTF-16 unit
// that falls back to the real byte length once it reaches the limit.
log("limit", Buffer.from("x".repeat(32767)).buffer.byteLength, Buffer.from("x".repeat(32768)).buffer.byteLength);
const euro = Buffer.from("€".repeat(9000));
log("estimate", euro.buffer.byteLength, euro.byteOffset, euro.length);
log("big", Buffer.allocUnsafe(40000).byteOffset, Buffer.allocUnsafe(40000).buffer.byteLength);

// A slice that does not fit starts a new pool.
for (let i = 0; i < 3000; i++) Buffer.allocUnsafe(30);
const late = Buffer.from("q");
log("refill", late.byteOffset, late.buffer === a.buffer, late.buffer.byteLength);
// With 24 bytes left, an 8-character string is estimated at 32 bytes and so
// starts a new pool, though its 8 real bytes would have fitted.
let cur = late.byteOffset + 8;
while (65536 - cur > 24 + 32000) {
  Buffer.allocUnsafe(32000);
  cur += 32000;
}
const filler = Buffer.allocUnsafe(65536 - cur - 24);
const est = Buffer.from("abcdefgh");
log("fill", filler.byteOffset + filler.length, est.byteOffset, est.buffer === late.buffer);

// Two of node's own producers that build their result with `allocUnsafe` or
// `concat`, and so return a pool slice.
const z = require("zlib");
log("producers", require("fs").readFileSync(__filename).buffer.byteLength, z.gzipSync("x").buffer.byteLength, z.gunzipSync(z.gzipSync("x")).buffer.byteLength);

// The pool cannot be transferred; an own buffer can.
try {
  a.buffer.transfer();
} catch (e) {
  log(e.name, e.message);
}
try {
  structuredClone(a.buffer, { transfer: [a.buffer] });
} catch (e) {
  log(e.name, e.message);
}
log("clone", structuredClone(a.buffer).byteLength, a.toString());
const own = Buffer.alloc(4, 1);
log("own", own.buffer.transfer().byteLength, own.length);

// Buffer.poolSize is read live for the limit; the current pool keeps its size.
Buffer.poolSize = 16;
log("poolSize", Buffer.from("abcdefg").buffer.byteLength, Buffer.from("abcdefgh").buffer.byteLength);

// Pooled Buffers are still equal by value.
log("equal", require("util").isDeepStrictEqual(Buffer.from("abc"), Buffer.alloc(3, "abc")), Buffer.from("abc").equals(Buffer.from("abc")), require("util").isDeepStrictEqual(Buffer.from("abc"), Buffer.from("abd")));
console.log(out.join("\n"));
