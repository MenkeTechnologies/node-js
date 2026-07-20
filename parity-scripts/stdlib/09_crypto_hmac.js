// crypto: createHmac sha256/sha1/md5 with fixed key + message.
const crypto = require("crypto");

const key = "secret-key";
const msg = "message to authenticate";

console.log(crypto.createHmac("sha256", key).update(msg).digest("hex"));
console.log(crypto.createHmac("sha1", key).update(msg).digest("hex"));
console.log(crypto.createHmac("md5", key).update(msg).digest("hex"));
console.log(crypto.createHmac("sha256", key).update(msg).digest("base64"));

// Chained update equivalence.
const h = crypto.createHmac("sha512", "k");
h.update("part1");
h.update("part2");
console.log(h.digest("hex"));

// Buffer key.
console.log(
  crypto.createHmac("sha256", Buffer.from("bytes-key")).update("data").digest("hex"),
);
