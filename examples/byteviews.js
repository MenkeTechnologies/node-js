// Byte views: the write-through mutators, the change-by-copy trio, the
// constructor's BYTES_PER_ELEMENT, the ToBigInt element coercion, and DataView
// index bounds. Every value here is a byte a text-shaped implementation would
// corrupt (0xff/0x80 are not valid UTF-8, 0x00 truncates a C-style read).

// `fill` writes THROUGH the view and answers the receiver, so a second view
// onto the same buffer sees it.
const a = new Uint8Array([0xff, 0xfe, 0x00, 0x41, 0x80]);
const ret = a.fill(9, 1, 3);
console.log('fill      ', a, ret === a);

const shared = new ArrayBuffer(4);
const lo = new Uint8Array(shared, 0, 2);
const hi = new Uint8Array(shared);
lo.fill(7);
console.log('through   ', hi, hi.buffer === lo.buffer);
console.log('relative  ', new Uint8Array(4).fill(1, -2), new Uint8Array([1, 2, 3]).fill(9, 5));
console.log('wrap      ', new Uint8Array([1, 2, 3]).fill(257, 0, 2), new Int8Array(2).fill(200));

// The change-by-copy trio leaves the receiver alone and answers a view of the
// receiver's own element kind — a Buffer receiver yields a Uint8Array.
const src = new Uint8Array([3, 1, 2]);
console.log('toSorted  ', src.toSorted(), src, src.toSorted((x, y) => y - x));
console.log('toReversed', src.toReversed(), new Float64Array([1.5, NaN]).toReversed());
console.log('with      ', src.with(0, 9), src.with(-1, 300), src);
console.log('fromBuffer', Buffer.from([1, 2, 3]).toReversed().constructor.name);
try {
  new Uint8Array([1, 2]).with(5, 1);
} catch (e) {
  console.log('withRange ', e.constructor.name, e.message);
}

// `BYTES_PER_ELEMENT` is a property of the constructor as well as the instance.
for (const C of [Uint8Array, Int16Array, Float64Array, BigInt64Array]) {
  console.log('bpe       ', C.name, C.BYTES_PER_ELEMENT, new C(2).byteLength);
}

// A 64-bit view's element write is ToBigInt, not "must already be a BigInt":
// booleans and strings convert, a Number does not, and each failure names the
// value it could not convert.
for (const v of [true, '12', '', [], ['3'], 5n, 1, 1.5, 'a', {}, null, undefined]) {
  const view = new BigInt64Array(1);
  let out;
  try {
    view[0] = v;
    out = String(view[0]);
  } catch (e) {
    out = `${e.constructor.name}: ${e.message}`;
  }
  console.log('tobigint  ', typeof v === 'bigint' ? v + 'n' : JSON.stringify(v) ?? String(v), '->', out);
}

// DataView index bounds: ToIndex truncates toward zero, and anything outside
// the view is a RangeError rather than a silent clamp.
const dv = new DataView(new Uint8Array([10, 20, 30, 40]).buffer);
console.log('dvIndex   ', dv.getUint8(1.9), dv.getUint8(-0.5), dv.getUint8(NaN));
for (const i of [-2, -1, 4, 100]) {
  try {
    dv.getUint8(i);
    console.log('dvBounds  ', i, 'no throw');
  } catch (e) {
    console.log('dvBounds  ', i, e.constructor.name, e.message);
  }
}
dv.setUint16(0, 0xbeef);
console.log('dvWrite   ', dv.getUint8(0), dv.getUint8(1), dv.getUint16(0), dv.getUint16(0, true));
