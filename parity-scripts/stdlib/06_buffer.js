// buffer: from / alloc / toString hex|base64|utf8 / byteLength / concat.
const { Buffer } = require("buffer");

const a = Buffer.from("Hello, World!", "utf8");
console.log(a.toString("hex"));
console.log(a.toString("base64"));
console.log(a.toString("utf8"));
console.log(a.length);
console.log(Buffer.byteLength("héllo", "utf8"));

const z = Buffer.alloc(5, 7);
console.log(z.toString("hex"));

const filled = Buffer.alloc(4).fill("ab");
console.log(filled.toString());

const cat = Buffer.concat([Buffer.from("foo"), Buffer.from("bar"), Buffer.from("baz")]);
console.log(cat.toString());

const fromHex = Buffer.from("48656c6c6f", "hex");
console.log(fromHex.toString("utf8"));

const fromB64 = Buffer.from("SGVsbG8=", "base64");
console.log(fromB64.toString("utf8"));
console.log(Buffer.from([0x62, 0x75, 0x66]).toString());
