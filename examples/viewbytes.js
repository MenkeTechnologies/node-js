// A byte-taking API reads a typed array as the BYTES it views, never as its
// element values truncated to one byte each. A `Uint16Array([0x4241])` is the
// two bytes `41 42`; every line below printed something else when an API took
// the element (`0x41`) or treated the view as empty. Values are chosen so the
// two readings differ.
const crypto = require("crypto"), buffer = require("buffer");
const { StringDecoder } = require("string_decoder");
const u16 = new Uint16Array([0x4241, 0x4443]);
const t = (label, f) => {
  try {
    console.log(label, JSON.stringify(f()));
  } catch (e) {
    console.log(label, e.name, e.code, e.message);
  }
};

t("hash      ", () => crypto.createHash("md5").update(u16).digest("hex"));
t("hmacKey   ", () => crypto.createHmac("sha1", new Uint16Array([0x4241])).update("x").digest("hex"));
t("isAscii   ", () => [buffer.isAscii(new Uint16Array([0x80])), buffer.isAscii(new Uint16Array([0x4241]))]);
t("isUtf8    ", () => buffer.isUtf8(new Uint8Array([0xc3, 0xa9]).buffer));
t("isUtf8Str ", () => buffer.isUtf8("a"));
t("isAsciiDV ", () => buffer.isAscii(new DataView(new ArrayBuffer(1))));
t("isLatin1  ", () => [buffer.isLatin1("ÿa"), buffer.isLatin1("€")]);
t("isLatin1Bf", () => buffer.isLatin1(Buffer.from("a")));

// TextDecoder takes any BufferSource and can stream a split character.
const td = new TextDecoder();
t("decode16  ", () => td.decode(u16));
t("decodeAB  ", () => td.decode(new Uint8Array([65, 66, 67]).buffer));
t("decodeDV  ", () => td.decode(new DataView(new Uint8Array([65, 66, 67]).buffer, 1)));
t("decodeStr ", () => td.decode("abc"));
t("utf16le   ", () => new TextDecoder("utf-16le").decode(new Uint16Array([0x41, 0x42])));
t("stream    ", () => [td.decode(new Uint8Array([0xe2, 0x82]), { stream: true }), td.decode(new Uint8Array([0xac]))]);
t("streamBOM ", () => {
  const d = new TextDecoder();
  return [d.decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x61]), { stream: true }), d.decode(new Uint8Array([0xef, 0xbb, 0xbf]))];
});

// TextEncoder.encodeInto writes whole characters only.
const te = new TextEncoder();
const dst = new Uint8Array(5);
t("encodeInto", () => [te.encodeInto("a€b", dst), Array.from(dst)]);
t("intoShort ", () => te.encodeInto("\u{1F600}x", new Uint8Array(3)));
t("intoU16   ", () => te.encodeInto("abc", new Uint16Array(4)));
t("intoSrc   ", () => te.encodeInto(5, new Uint8Array(4)));
t("encodeNone", () => [te.encode().length, te.encode(undefined).length, te.encode(null).length]);

// Blob parts, and a StringDecoder chunk.
t("blob      ", () => new Blob([u16, new ArrayBuffer(3), new DataView(new ArrayBuffer(2)), 5, null, undefined]).size);
t("decoder   ", () => new StringDecoder().write(u16));
t("decoderHex", () => new StringDecoder("hex").end(new Uint16Array([0x4241])));
t("decoderArr", () => new StringDecoder().write([65]));
// A string chunk is already text: it passes through while held-back bytes stay
// held, so the flush comes after it.
t("decoderEnd", () => { const d = new StringDecoder(); return [d.write(new Uint8Array([0xe2])), d.end("xy")]; });

// Buffer methods that take another view.
const b = Buffer.alloc(4);
b.set(new Uint16Array([0x4241, 7]));
t("set16     ", () => b.toString("hex"));
b.set([1, 2], 2);
t("setArray  ", () => b.toString("hex"));
t("setRange  ", () => b.set([1], 4));
t("equals16  ", () => Buffer.from("ABCD").equals(u16));
t("indexOf16 ", () => Buffer.from("ABCD").indexOf(u16));
t("indexOfArr", () => Buffer.from("ab").indexOf([97]));
t("indexOfNum", () => [Buffer.from("abab").indexOf(353), Buffer.from("abab").indexOf(-159)]);
t("copyFrom  ", () => [Buffer.copyBytesFrom(u16).toString(), Buffer.copyBytesFrom(u16, 1).toString(), Buffer.copyBytesFrom(u16, 0, 1).toString()]);
t("copyFromAB", () => Buffer.copyBytesFrom(new ArrayBuffer(2)));
t("copyOffset", () => Buffer.copyBytesFrom(u16, 1.5));

// compare's four bounds, and its operand check.
const src = Buffer.from("abcdef"), dstB = Buffer.from("xbcdz");
t("compare   ", () => [src.compare(dstB, 1, 3, 1, 3), src.compare(dstB, 1, 3, 1), src.compare(dstB, 9), src.compare(dstB, 3, 1), src.compare(dstB, 1, 3, 7)]);
t("compareEnd", () => src.compare(dstB, 1, 9));
t("compareTyp", () => src.compare(dstB, "1"));
t("compareStr", () => src.compare("x"));
t("compareSt ", () => Buffer.compare(Buffer.from("a"), 5));

// hex stops at the first pair that is not two hex digits.
t("hex       ", () => ["zz12", "12zz34", "123", "ABcd"].map((h) => Buffer.from(h, "hex").toString("hex")));

// atob is forgiving-base64; btoa refuses a code point above U+00FF.
t("btoa      ", () => btoa("€"));
t("btoaToStr ", () => [btoa(undefined), btoa({ toString() { return "ab"; } })]);
t("btoaNone  ", () => btoa());
for (const s of ["aGk", "aG k=\n", "a", "a=", "ab=", "abcde=", "abcd=", "a!b=", "YR=="]) t(`atob ${JSON.stringify(s)}`, () => atob(s));

t("constants ", () => [buffer.kMaxLength, buffer.kStringMaxLength, buffer.constants, buffer.INSPECT_MAX_BYTES]);

// randomFillSync fills any view, scaling offset/size by the element width;
// only the shape of the result is stable, not the random bytes.
const r = new Uint32Array(4);
crypto.randomFillSync(r, 1, 2);
t("fill32    ", () => [r[0], r[3]]);
const ab = new ArrayBuffer(8), abView = new Uint8Array(ab);
crypto.randomFillSync(ab, 4);
t("fillAB    ", () => Array.from(abView.slice(0, 4)));
t("fillRange ", () => crypto.randomFillSync(new Uint32Array(4), 5));
t("fillSize  ", () => crypto.randomFillSync(new Uint32Array(4), 1, 4));
t("fillStr   ", () => crypto.randomFillSync("x"));
t("fillCb    ", () => crypto.randomFill(Buffer.alloc(2)));
crypto.randomFill(new Uint16Array(2), (e, out) => console.log("fillAsync ", e, out.constructor.name, out.length));
