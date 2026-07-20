// crypto: createHash md5/sha1/sha256/sha512 digest of fixed input.
const crypto = require("crypto");

const input = "The quick brown fox jumps over the lazy dog";

console.log(crypto.createHash("md5").update(input).digest("hex"));
console.log(crypto.createHash("sha1").update(input).digest("hex"));
console.log(crypto.createHash("sha256").update(input).digest("hex"));
console.log(crypto.createHash("sha512").update(input).digest("hex"));

console.log(crypto.createHash("sha256").update(input).digest("base64"));
console.log(crypto.createHash("md5").update("").digest("hex"));

// Chained updates must equal single update.
const h = crypto.createHash("sha256");
h.update("The quick brown fox ");
h.update("jumps over the lazy dog");
console.log(h.digest("hex"));

// Binary / utf8 input.
console.log(crypto.createHash("sha1").update(Buffer.from("abc")).digest("hex"));
