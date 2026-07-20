// buffer: indexOf / lastIndexOf / includes / toJSON / iteration.
const { Buffer } = require("buffer");

const buf = Buffer.from("this is a test buffer for searching");

console.log(buf.indexOf("is"));
console.log(buf.indexOf("test"));
console.log(buf.indexOf("missing"));
console.log(buf.lastIndexOf("is"));
console.log(buf.includes("buffer"));
console.log(buf.includes("xyz"));
console.log(buf.indexOf(0x74)); // 't'

const small = Buffer.from([1, 2, 3]);
console.log(JSON.stringify(small.toJSON()));
console.log(JSON.stringify(small));

console.log([...small].join(","));
console.log([...small.values()].join(","));
console.log([...small.keys()].join(","));
console.log(Buffer.isBuffer(buf), Buffer.isBuffer("no"));
