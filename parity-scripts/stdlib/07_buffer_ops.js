// buffer: compare / equals / slice / write / readUInt / writeUInt.
const { Buffer } = require("buffer");

const a = Buffer.from("ABCDEF");
const b = Buffer.from("ABCDEF");
const c = Buffer.from("ABCDEG");

console.log(a.equals(b));
console.log(a.equals(c));
console.log(a.compare(c));
console.log(c.compare(a));
console.log(Buffer.compare(a, b));

const sliced = a.subarray(1, 4);
console.log(sliced.toString());

const buf = Buffer.alloc(8);
buf.writeUInt8(255, 0);
buf.writeUInt16BE(4660, 1);
buf.writeUInt32BE(305419896, 3);
console.log(buf.toString("hex"));
console.log(buf.readUInt8(0));
console.log(buf.readUInt16BE(1));
console.log(buf.readUInt32BE(3));

const w = Buffer.alloc(10);
const written = w.write("hello", 2, "utf8");
console.log(written, w.toString("hex"));
