// A native constructor's `.prototype` is a real object, and the ES5 subclassing
// pattern built on it works.
//
// The subclass below is `iconv-lite`'s internal codec, reduced: it adopts
// `StringDecoder.prototype` wholesale and initializes itself by CALLING the
// native constructor with its own `this`. Two facts have to hold at once and
// pull in opposite directions — the prototype has to be a real shared object
// (so the subclass inherits the methods) and the call has to initialize the
// SUPPLIED object rather than return a fresh one (so the state lands where the
// methods look for it). Neither alone makes `d.write(...)` decode anything.
const { StringDecoder } = require("string_decoder");

console.log(typeof StringDecoder.prototype, typeof StringDecoder.prototype.write, typeof StringDecoder.prototype.end);
console.log(StringDecoder.prototype === StringDecoder.prototype);

const direct = new StringDecoder("utf8");
console.log(typeof direct.write, typeof direct.end);
console.log(direct.write(Buffer.from("hi")) + direct.end());

// The `iconv-lite` shape.
function Sub(enc) {
  StringDecoder.call(this, enc);
}
Sub.prototype = StringDecoder.prototype;
const d = new Sub("utf8");
console.log(typeof d.write, typeof d.end);

// A multi-byte sequence split across chunks must be held back and completed —
// a decoder whose state did not land on `this` returns the wrong text here.
const bytes = Buffer.from("é", "utf8");
console.log(bytes.length, JSON.stringify(d.write(bytes.slice(0, 1))), JSON.stringify(d.write(bytes.slice(1))));
console.log(JSON.stringify(d.end()));

// A second instance must not share the first's held-back bytes.
const d2 = new Sub("utf8");
console.log(JSON.stringify(d2.write(Buffer.from("ok"))));
