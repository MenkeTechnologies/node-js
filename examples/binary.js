// ArrayBuffer, typed-array views and DataView. An `ArrayBuffer` used to carry
// nothing but a `byteLength` and each typed array owned a private element
// vector, so no two views ever shared memory, `DataView` did not exist, and
// `Buffer.from(arrayBuffer)` produced that many ZERO bytes.

// Two views over one buffer see each other's writes — the whole point.
const ab = new ArrayBuffer(8);
const u8 = new Uint8Array(ab), u32 = new Uint32Array(ab);
u8[0] = 1; u8[1] = 2;
console.log("shared  ", u32[0], new Uint8Array(ab)[1], u8.buffer === ab, u32.buffer === ab);
u32[1] = 0x01020304;
console.log("back    ", u8[4], u8[5], u8[6], u8[7]);

// A view can start partway into the buffer and cover part of it.
const part = new Uint8Array(ab, 4, 2);
console.log("offset  ", part.length, part.byteOffset, part.byteLength, part[0]);
part[0] = 9;
console.log("aliased ", u8[4], new Uint8Array(ab, 4, 2)[0]);
// An unaligned or over-long window is refused.
const fail = (f) => { try { f(); return "ok"; } catch (e) { return e.constructor.name; } };
console.log("bounds  ", fail(() => new Uint32Array(ab, 2)), fail(() => new Uint8Array(ab, 0, 99)),
  fail(() => new Uint8Array(ab, 0, 8)));

// `subarray` is a VIEW (writes show through); `slice` copies.
const base = new Uint8Array([1, 2, 3, 4]);
const sub = base.subarray(1, 3), cut = base.slice(1, 3);
sub[0] = 99; cut[1] = 77;
console.log("subarray", base.join(","), sub.join(","), cut.join(","), sub.buffer === base.buffer);

// DataView reads and writes at byte granularity, BIG-endian by default —
// which is what distinguishes it from a typed array.
const dv = new DataView(new ArrayBuffer(8));
dv.setUint16(0, 0x1234);
console.log("dataview", dv.getUint16(0), dv.getUint16(0, true), dv.getUint8(0), dv.getUint8(1));
dv.setFloat64(0, 1.5);
console.log("float   ", dv.getFloat64(0), dv.byteLength, dv.byteOffset);
dv.setBigInt64(0, -2n);
console.log("bigint  ", dv.getBigInt64(0), dv.getBigUint64(0));
console.log("dvbounds", fail(() => new DataView(new ArrayBuffer(2)).getUint32(0)),
  fail(() => new DataView(new ArrayBuffer(2), 3)), fail(() => new DataView({})));
// A DataView over a window of a buffer writes into that window only.
const shared = new ArrayBuffer(4);
new DataView(shared, 1, 2).setUint16(0, 0xabcd);
console.log("dvwindow", [...new Uint8Array(shared)].join(","));

// `ArrayBuffer.prototype.slice` copies; `isView` distinguishes a view from the
// buffer it looks at.
const cutBuf = ab.slice(0, 2);
new Uint8Array(cutBuf)[0] = 200;
console.log("abslice ", cutBuf.byteLength, u8[0], ab.slice(2).byteLength);
console.log("isView  ", ArrayBuffer.isView(u8), ArrayBuffer.isView(dv), ArrayBuffer.isView(ab), ArrayBuffer.isView([]));

// A Buffer over an ArrayBuffer shares its memory too, window included.
const shareAb = new ArrayBuffer(4);
const wholeBuf = Buffer.from(shareAb);
wholeBuf[0] = 7;
const windowBuf = Buffer.from(shareAb, 2, 2);
windowBuf[0] = 8;
console.log("bufshare", [...new Uint8Array(shareAb)].join(","), wholeBuf.length, windowBuf.length, windowBuf.byteOffset);

// Element kinds round-trip through the shared bytes.
console.log("kinds   ", new Int8Array([-1])[0], new Int16Array([-2])[0], new Int32Array([-3])[0],
  new Float32Array([1.5])[0], new Float64Array([1.5])[0], new Uint8ClampedArray([300])[0]);
console.log("bigkinds", new BigInt64Array([-1n])[0], new BigUint64Array([1n])[0]);

// A resizable buffer grows in place, so existing views see the new size.
const grow = new ArrayBuffer(2, { maxByteLength: 8 });
console.log("resize  ", grow.resizable, grow.maxByteLength, grow.byteLength);
grow.resize(6);
console.log("grown   ", grow.byteLength, new Uint8Array(grow).length, fail(() => grow.resize(99)));

// Branding, inspection and iteration all read the shared store.
console.log("brand   ", Object.prototype.toString.call(ab), Object.prototype.toString.call(dv),
  Object.prototype.toString.call(u8), u8 instanceof Uint8Array);
console.log("inspect ", new ArrayBuffer(3), new Uint8Array([1, 2]), new Int32Array([5]),
  new BigInt64Array([7n]));
console.log("iterate ", [...new Uint8Array([1, 2, 3])].join("|"), Object.keys(new Uint8Array(2)).join(","),
  Array.from(new Int16Array([4, 5])).join("|"), JSON.stringify(new Uint8Array([1, 2])));
