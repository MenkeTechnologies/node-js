// The `Uint8Array` base64/hex methods — the conversions that no longer need a
// `Buffer`. All six were absent.
const show = (label, f) => {
  try {
    const r = f();
    const out = ArrayBuffer.isView(r) ? [...r].join(",")
      : (typeof r === "object" && r !== null ? JSON.stringify(r) : JSON.stringify(r));
    console.log(label, out);
  } catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const u = new Uint8Array([72, 101, 108, 108, 111, 255, 0]);
console.log("shape     ", typeof Uint8Array.fromBase64, typeof Uint8Array.fromHex,
  typeof u.toBase64, typeof u.toHex, typeof u.setFromBase64, typeof u.setFromHex);
console.log("arities   ", Uint8Array.fromBase64.length, Uint8Array.fromHex.length,
  u.toBase64.length, u.toHex.length, u.setFromBase64.length, u.setFromHex.length);
// They live on `Uint8Array` alone — no other view has them.
console.log("uint8-only", typeof Uint16Array.fromBase64, typeof Uint16Array.prototype.toBase64,
  typeof Uint8ClampedArray.prototype.toHex);
console.log("own-names ", JSON.stringify(Object.getOwnPropertyNames(Uint8Array.prototype)));

console.log("toBase64  ", JSON.stringify(u.toBase64()), JSON.stringify(u.toBase64({ alphabet: "base64url" })));
// The url alphabet swaps two characters; it does NOT drop the padding, which is
// a separate option.
console.log("omitPad   ", JSON.stringify(new Uint8Array([1]).toBase64({ omitPadding: true })),
  JSON.stringify(new Uint8Array([1, 2]).toBase64({ omitPadding: true })),
  JSON.stringify(new Uint8Array([1, 2, 3]).toBase64({ omitPadding: true })));
console.log("toHex     ", JSON.stringify(u.toHex()), JSON.stringify(new Uint8Array(0).toHex()),
  JSON.stringify(new Uint8Array(0).toBase64()));
console.log("from      ", [...Uint8Array.fromBase64("SGVsbG8=")].join(","),
  [...Uint8Array.fromBase64("SGVsbG__AA==", { alphabet: "base64url" })].join(","),
  [...Uint8Array.fromHex("48656C6c6f")].join(","));

// `setFrom*` writes as much as FITS and reports how far it got — a short target
// is not an error, it stops at the last chunk that fits.
const four = new Uint8Array(4);
console.log("setFrom   ", JSON.stringify(four.setFromBase64("SGVsbG8=")), [...four].join(","));
const nine = new Uint8Array(9);
console.log("setFrom-big", JSON.stringify(nine.setFromBase64("SGVsbG8=")), [...nine].join(","));
const three = new Uint8Array(3);
console.log("setFromHex", JSON.stringify(three.setFromHex("4865")), [...three].join(","));

// Decoding is STRICT — the lenient `atob` decoder cannot serve here. A stray
// `=`, a wrong pad count and a character outside the selected alphabet are all
// rejected, while ASCII whitespace between characters is skipped.
for (const text of ["!!", "SGVsbG8", "S", "SGV=sbG8=", "", "=", "AA===", "A===",
  "SGVsbG8=x", "SG Vs", "SGVsbG8==", "QQ==", "QQ=", "__8="]) {
  show("b64 " + JSON.stringify(text), () => Uint8Array.fromBase64(text));
}
// `lastChunkHandling` decides what a trailing PARTIAL chunk means. The default
// is `loose`, which is why an unpadded string decodes at all.
for (const handling of ["loose", "strict", "stop-before-partial", "bogus"]) {
  show("lch " + handling + " unpadded", () => Uint8Array.fromBase64("SGVsbG8", { lastChunkHandling: handling }));
  show("lch " + handling + " padded  ", () => Uint8Array.fromBase64("QQ==", { lastChunkHandling: handling }));
}
// Hex reports the same message for an odd length and for a non-hex character.
for (const text of ["0", "zz", "gg", "", "0011", "0x11", " 00", "00 "]) {
  show("hex " + JSON.stringify(text), () => Uint8Array.fromHex(text));
}
show("nonstring ", () => Uint8Array.fromBase64(5));
show("nonstring2", () => Uint8Array.fromHex(5));
show("nonstring3", () => new Uint8Array(2).setFromHex(5));
show("badAlpha  ", () => u.toBase64({ alphabet: "x" }));
show("nullOpts  ", () => u.toBase64(null));
// The brand check is against `Uint8Array` specifically: another view is as
// incompatible as a plain object, and each renders differently.
show("wrongView ", () => Uint8Array.prototype.toBase64.call(new Int8Array(1)));
show("array     ", () => Uint8Array.prototype.toHex.call([1]));
show("object    ", () => Uint8Array.prototype.toHex.call({}));

// Round trip.
const bytes = new Uint8Array([0, 1, 127, 128, 254, 255]);
console.log("roundtrip ", [...Uint8Array.fromBase64(bytes.toBase64())].join(",") === [...bytes].join(","),
  [...Uint8Array.fromHex(bytes.toHex())].join(",") === [...bytes].join(","));
