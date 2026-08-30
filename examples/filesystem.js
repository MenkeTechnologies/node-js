// `fs` encoding handling. Only the string-argument utf8 form worked: the
// options-object form was discarded outright, and every other encoding came
// back as the file's raw UTF-8 text on read, or was written literally.
//
// Nothing path-dependent is printed, and the scratch directory is named after
// this process so concurrent runs cannot collide.
const fs = require("fs");
const os = require("os");
const path = require("path");

const dir = path.join(os.tmpdir(), "node-js-fs-record-" + process.pid);
fs.rmSync(dir, { recursive: true, force: true });
fs.mkdirSync(dir, { recursive: true });
const file = path.join(dir, "a.txt");
fs.writeFileSync(file, "hello");

// Reading with no encoding gives a Buffer; with one, the encoding names the
// REPRESENTATION to return — not merely "decode this as text".
const shown = (v) => (Buffer.isBuffer(v) ? "Buffer<" + v.toString("hex") + ">" : JSON.stringify(v));
console.log("none    ", shown(fs.readFileSync(file)));
console.log("utf8    ", shown(fs.readFileSync(file, "utf8")), shown(fs.readFileSync(file, { encoding: "utf8" })));
console.log("hex     ", shown(fs.readFileSync(file, "hex")), shown(fs.readFileSync(file, { encoding: "hex" })));
console.log("base64  ", shown(fs.readFileSync(file, "base64")), shown(fs.readFileSync(file, "latin1")));
// `{ encoding: null }` really does mean "no encoding", and other options keys
// alongside it must not stop the encoding being seen.
console.log("null-opt", shown(fs.readFileSync(file, { encoding: null })), shown(fs.readFileSync(file, { encoding: "utf8", flag: "r" })));

// Writing works the other way: the DATA STRING is in the given encoding and has
// to be decoded first. `writeFileSync(p, '68656c6c6f', 'hex')` writes five
// bytes, not the ten digits — those digits used to land on disk verbatim.
fs.writeFileSync(file, "68656c6c6f", "hex");
console.log("wr-hex  ", shown(fs.readFileSync(file, "utf8")));
fs.writeFileSync(file, "aGk=", { encoding: "base64" });
console.log("wr-b64  ", shown(fs.readFileSync(file, "utf8")));
fs.writeFileSync(file, "plain");
fs.appendFileSync(file, "21", "hex");
console.log("ap-hex  ", shown(fs.readFileSync(file, "utf8")));
// A Buffer argument carries its own bytes and ignores any encoding.
fs.writeFileSync(file, Buffer.from([104, 105]), "hex");
console.log("wr-buf  ", shown(fs.readFileSync(file, "utf8")));

// The async callback read had the same defect; the promises one did not.
fs.writeFileSync(file, "hello");
fs.readFile(file, "hex", (err, data) => {
  console.log("cb-hex  ", String(err), shown(data));
  fs.readFile(file, { encoding: "base64" }, (err2, data2) => {
    console.log("cb-b64  ", String(err2), shown(data2));
    fs.readFile(file, (err3, data3) => {
      console.log("cb-buf  ", String(err3), Buffer.isBuffer(data3));
      require("fs/promises").readFile(file, "hex").then((d) => {
        console.log("prom-hex", shown(d));
        fs.rmSync(dir, { recursive: true, force: true });
        console.log("cleanup ", fs.existsSync(dir));
      });
    });
  });
});
