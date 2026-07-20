// zlib: gzip/deflate/brotli round-trips — assert INPUT recovered, not compressed bytes.
const zlib = require("zlib");

const input = "The quick brown fox jumps over the lazy dog. ".repeat(10);

const gz = zlib.gzipSync(input);
console.log(zlib.gunzipSync(gz).toString() === input);

const df = zlib.deflateSync(input);
console.log(zlib.inflateSync(df).toString() === input);

const raw = zlib.deflateRawSync(input);
console.log(zlib.inflateRawSync(raw).toString() === input);

const br = zlib.brotliCompressSync(input);
console.log(zlib.brotliDecompressSync(br).toString() === input);

// Round-trip preserves multibyte content.
const uni = "héllo 中文 😀 café";
console.log(zlib.gunzipSync(zlib.gzipSync(uni)).toString() === uni);

// Compressed output is smaller than input for repetitive data.
console.log(gz.length < Buffer.byteLength(input));

// Round-trip of empty string.
console.log(zlib.gunzipSync(zlib.gzipSync("")).toString() === "");
