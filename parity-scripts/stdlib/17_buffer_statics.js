// `Buffer`'s static surface, and the deprecated `Buffer(...)` call form.
//
// The feature-detection block at the bottom is the one `safe-buffer` runs. It
// tests four statics together, so a single missing one flips the whole package
// onto its legacy `SafeBuffer` wrapper — which express's `res.send` then uses
// for every response. A probe that checked the statics one at a time would not
// show that; this one reproduces the actual conjunction.
console.log(
  ["from", "alloc", "allocUnsafe", "allocUnsafeSlow", "concat", "isBuffer", "isEncoding", "byteLength", "compare", "of"]
    .map((n) => (typeof Buffer[n] === "function" ? 1 : 0))
    .join("")
);

// `of` takes one byte per argument.
console.log(Buffer.of(1, 2, 3).toString("hex"), Buffer.of().length, Buffer.of(255).toString("hex"));

// `allocUnsafeSlow` sizes like `allocUnsafe`; fill so the contents are defined.
const slow = Buffer.allocUnsafeSlow(4);
slow.fill(0);
console.log(slow.length, slow.toString("hex"));

// `isEncoding` is case-insensitive over exactly Node's set.
console.log(
  ["utf8", "utf-8", "UTF8", "UTF-8", "ucs2", "ucs-2", "utf16le", "utf-16le", "latin1", "binary",
   "base64", "base64url", "hex", "ascii", "ASCII", "Hex", "utf7", "utf-16be", "none", ""]
    .map((c) => (Buffer.isEncoding(c) ? 1 : 0))
    .join("")
);

// The `Buffer(x)` call form and `new Buffer(x)` are the same operation, and a
// NUMBER allocates that many zero bytes rather than going through `from` (where
// `Buffer.from(3)` is a TypeError). The number and the array below hold the same
// digits and demand different results.
console.log(Buffer("abc").toString(), Buffer([1, 2]).toString("hex"), Buffer(3).length, Buffer(3).toString("hex"));
console.log(new Buffer("abc").toString(), new Buffer([1, 2]).toString("hex"), new Buffer(3).length);
console.log(Buffer(3).length === Buffer.alloc(3).length, Buffer([3]).length);

// The exact conjunction `safe-buffer` evaluates.
const buffer = require("buffer");
console.log(!!(buffer.Buffer.from && buffer.Buffer.alloc && buffer.Buffer.allocUnsafe && buffer.Buffer.allocUnsafeSlow));
// That conjunction is satisfied by any four truthy values, so it passes whether
// or not the four do anything. Each is now also CALLED and its result compared.
console.log(buffer.Buffer.from("ab").toString("hex"), buffer.Buffer.alloc(2).toString("hex"));
console.log(buffer.Buffer.allocUnsafe(3).length, buffer.Buffer.allocUnsafeSlow(3).length);
console.log(buffer.Buffer === Buffer, buffer.Buffer.from("ab").equals(Buffer.from("ab")));

// A fixed-width read past the end THROWS; it used to answer 0, which is
// indistinguishable from a buffer that really holds a zero there.
const four = Buffer.from([1, 2, 3, 4]);
console.log(four.readUInt8(3), four.readUInt16BE(2), four.readInt32BE(0));
for (const [label, f] of [
  ["past-end", () => four.readUInt8(4)],
  ["negative", () => four.readUInt8(-1)],
  ["no-room", () => four.readUInt32LE(1)],
  ["fractional", () => four.readUInt8(1.5)],
  ["empty-buffer", () => Buffer.alloc(0).readUInt8(0)],
]) {
  try {
    console.log(label, "=", f());
  } catch (e) {
    console.log(label, e.name, e.code, e.message);
  }
}
