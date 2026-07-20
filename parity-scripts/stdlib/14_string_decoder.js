// string_decoder: write / end across chunk boundaries incl multibyte utf8.
const { StringDecoder } = require("string_decoder");

const decoder = new StringDecoder("utf8");

// "€" is 3 bytes: e2 82 ac — split across writes.
const euro = Buffer.from("€", "utf8");
let out = "";
out += decoder.write(euro.subarray(0, 1));
out += decoder.write(euro.subarray(1, 2));
out += decoder.write(euro.subarray(2, 3));
console.log(JSON.stringify(out));

const d2 = new StringDecoder("utf8");
const cat = Buffer.from("¢中文", "utf8");
let acc = "";
for (const byte of cat) {
  acc += d2.write(Buffer.from([byte]));
}
acc += d2.end();
console.log(acc);

const d3 = new StringDecoder("utf8");
console.log(d3.write(Buffer.from("hello")));
console.log(JSON.stringify(d3.end()));

const d4 = new StringDecoder("utf8");
const emoji = Buffer.from("😀", "utf8");
let e = d4.write(emoji.subarray(0, 2));
e += d4.write(emoji.subarray(2));
console.log(e);
