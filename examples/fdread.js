// fs.readSync/read/readvSync fill whatever VIEW they are given, at a BYTE
// offset, with node's argument forms and validation. A typed array other than
// a Buffer used to receive nothing, the options-object form ignored its fields,
// a `position` moved the descriptor's own offset, and a bad argument reached the
// callback instead of throwing.
const fs = require("fs"), path = require("path"), os = require("os");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "njfd-"));
const file = path.join(dir, "f");
fs.writeFileSync(file, "ABCDEFGH");
const fd = fs.openSync(file);
const t = (label, f) => {
  try {
    console.log(label, ...f());
  } catch (e) {
    console.log(label, e.name, e.code, e.message);
  }
};

const u16 = new Uint16Array(3);
t("u16       ", () => [fs.readSync(fd, u16, 1, 4, 0), u16]);
const dv = new DataView(new ArrayBuffer(4));
t("dataView  ", () => [fs.readSync(fd, dv, 0, 4, 2), dv.getUint32(0).toString(16)]);
t("arrayBuf  ", () => [fs.readSync(fd, new ArrayBuffer(4), 0, 4, 0)]);
t("tooLong   ", () => [fs.readSync(fd, Buffer.alloc(2), 0, 4, 0)]);
t("offsetPast", () => [fs.readSync(fd, Buffer.alloc(2), 3, 1, 0)]);
t("empty     ", () => [fs.readSync(fd, Buffer.alloc(0), 0, 1, 0)]);
t("emptyZero ", () => [fs.readSync(fd, Buffer.alloc(0), 0, 0, 0)]);
t("offsetStr ", () => [fs.readSync(fd, Buffer.alloc(2), "1", 1, 0)]);
t("offsetFrac", () => [fs.readSync(fd, Buffer.alloc(2), 1.5, 1, 0)]);
t("lengthNeg ", () => [fs.readSync(fd, Buffer.alloc(2), 0, -1, 0)]);
t("posNeg    ", () => [fs.readSync(fd, Buffer.alloc(2), 0, 1, -2)]);
t("posStr    ", () => [fs.readSync(fd, Buffer.alloc(2), 0, 1, "3")]);
t("optsNum   ", () => [fs.readSync(fd, Buffer.alloc(2), 0)]);

// A position reads with pread: the descriptor's own offset does not move, so a
// following position-less read starts where the previous one left off.
const p1 = Buffer.alloc(2), p2 = Buffer.alloc(2);
fs.readSync(fd, p1, 0, 2, 4);
fs.readSync(fd, p2, 0, 2, null);
t("pread     ", () => [p1.toString(), p2.toString()]);

// The options-object form, including a string length that `| 0` coerces.
const o1 = Buffer.alloc(4), o2 = Buffer.alloc(4), o3 = Buffer.alloc(2);
t("optsPos   ", () => [fs.readSync(fd, o1, { position: 4 }), o1.toString()]);
t("optsAll   ", () => [fs.readSync(fd, o2, { offset: 1, length: 2, position: 0 }), o2]);
t("optsLenStr", () => [fs.readSync(fd, o3, { length: "1", position: 7 }), o3]);

// readvSync fills each view in turn.
const vs = [Buffer.alloc(2), new Uint8Array(3)];
t("readv     ", () => [fs.readvSync(fd, vs, 0), vs]);

// fs.read: argument errors throw synchronously; the callback gets the view.
t("readNoCb  ", () => [fs.read(fd, Buffer.alloc(2), 0, 2, 0)]);
t("readAB    ", () => [fs.read(fd, new ArrayBuffer(2), 0, 2, 0, () => {})]);
fs.read(fd, new Uint16Array(2), 0, 4, 0, (e, n, b) => {
  console.log("read      ", e, n, b);
  fs.read(fd, { buffer: Buffer.alloc(3), position: 5 }, (e2, n2, b2) => {
    console.log("readParams", e2, n2, b2.toString());
    fs.read(fd, Buffer.alloc(2), { position: 6 }, (e3, n3, b3) => {
      console.log("readOpts  ", e3, n3, b3.toString());
      fs.closeSync(fd);
      fs.rmSync(dir, { recursive: true });
    });
  });
});
