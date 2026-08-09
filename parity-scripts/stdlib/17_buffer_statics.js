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
