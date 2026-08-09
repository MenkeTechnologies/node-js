// A Buffer is a real Uint8Array subclass, and the typed arrays it inherits from
// behave like Node's. Every assertion here is a site that must agree once that
// is true — the failure mode this guards against is the invariant holding at
// SOME of them and not the rest, which is exactly how it was: `instanceof`
// passed for an instance but not for `Buffer.prototype`, `hasOwnProperty(0)`
// knew about byte indices but `0 in buf` did not, and `Buffer.from` understood
// another Buffer but turned any other typed array into the bytes of
// "[object Object]".

// ── the subclass link, both halves ──────────────────────────────────────────
const buf = Buffer.from([104, 105, 195, 188]);
console.log('proto instanceof U8   :', Buffer.prototype instanceof Uint8Array);
console.log('instance instanceof U8:', buf instanceof Uint8Array);
console.log('instance instanceof B :', buf instanceof Buffer);
console.log('getProto(buf)         :', Object.getPrototypeOf(buf) === Buffer.prototype);
console.log('getProto(B.prototype) :', Object.getPrototypeOf(Buffer.prototype) === Uint8Array.prototype);
// The class side of `class Buffer extends Uint8Array`.
console.log('getProto(Buffer)      :', Object.getPrototypeOf(Buffer) === Uint8Array);
// %TypedArray%.prototype sits between Uint8Array.prototype and Object.prototype,
// so this is false in Node; the shared methods live on that intermediate.
const TAp = Object.getPrototypeOf(Uint8Array.prototype);
console.log('U8proto -> Objproto   :', Object.getPrototypeOf(Uint8Array.prototype) === Object.prototype);
console.log('TAproto -> Objproto   :', Object.getPrototypeOf(TAp) === Object.prototype);
console.log('U8proto owns every    :', Uint8Array.prototype.hasOwnProperty('every'));
console.log('TAproto owns every    :', TAp.hasOwnProperty('every'));
console.log('U8proto owns BPE      :', Uint8Array.prototype.hasOwnProperty('BYTES_PER_ELEMENT'));

// ── brands and predicates ───────────────────────────────────────────────────
console.log('isArray               :', Array.isArray(buf));
console.log('isView                :', ArrayBuffer.isView(buf));
console.log('isBuffer(buf)         :', Buffer.isBuffer(buf));
console.log('isBuffer(u8)          :', Buffer.isBuffer(new Uint8Array(2)));
console.log('brand                 :', Object.prototype.toString.call(buf));
// The brand is readable as a property, not only via Object.prototype.toString.
console.log('toStringTag buf       :', buf[Symbol.toStringTag]);
console.log('toStringTag u8        :', new Uint8Array(1)[Symbol.toStringTag]);
console.log('toStringTag i32       :', new Int32Array(1)[Symbol.toStringTag]);
// A legacy builtin brands but exposes no such property.
console.log('toStringTag array     :', [][Symbol.toStringTag]);
console.log('toStringTag date      :', new Date()[Symbol.toStringTag]);

// ── index keys: `in` and hasOwnProperty must give the same answer ───────────
console.log('0 in buf              :', 0 in buf);
console.log('3 in buf              :', 3 in buf);
console.log('4 in buf              :', 4 in buf);
console.log('hasOwn 0 buf          :', buf.hasOwnProperty(0));
console.log('0 in u8               :', 0 in new Uint8Array(2));
console.log('2 in u8               :', 2 in new Uint8Array(2));
console.log('length in buf         :', 'length' in buf);
console.log('keys                  :', Object.keys(buf).join(','));

// ── view metadata ───────────────────────────────────────────────────────────
console.log('byteLength/byteOffset :', buf.byteLength, buf.byteOffset, buf.BYTES_PER_ELEMENT);
const u16 = new Uint16Array([1, 2, 3]);
console.log('u16 view metadata     :', u16.length, u16.byteLength, u16.byteOffset, u16.BYTES_PER_ELEMENT);

// ── any typed array is a byte source, not just another Buffer ───────────────
console.log('from(u8)              :', [...Buffer.from(new Uint8Array([65, 66, 300 & 255]))].join(','));
console.log('from(i32) truncates   :', [...Buffer.from(new Int32Array([1, 2, 300]))].join(','));
console.log('from(arraybuffer)     :', [...Buffer.from(new ArrayBuffer(4))].join(','));
console.log('concat mixed          :', [...Buffer.concat([new Uint8Array([1, 2]), Buffer.from([3])])].join(','));
// byteLength is the VIEW size, which is not the element count for a wider kind.
console.log('byteLength(u8)        :', Buffer.byteLength(new Uint8Array([1, 2, 3])));
console.log('byteLength(i32)       :', Buffer.byteLength(new Int32Array([1, 2, 3])));
console.log('equals(u8)            :', Buffer.from([1, 2]).equals(new Uint8Array([1, 2])));
console.log('indexOf(u8)           :', Buffer.from([1, 2, 3]).indexOf(new Uint8Array([2, 3])));

// ── inherited typed-array methods work on a Buffer AND keep its type ────────
const mk = () => Buffer.from([3, 1, 2]);
console.log('every/some            :', mk().every(x => x > 0), mk().some(x => x > 2));
console.log('map                   :', [...mk().map(x => x * 2)].join(','));
console.log('filter                :', [...mk().filter(x => x > 1)].join(','));
console.log('find/findIndex        :', mk().find(x => x > 1), mk().findIndex(x => x > 1));
console.log('findLast/Index        :', mk().findLast(x => x > 1), mk().findLastIndex(x => x > 1));
console.log('reduce/reduceRight    :', mk().reduce((a, x) => a + x, 0), mk().reduceRight((a, x) => a + x, 0));
console.log('join                  :', mk().join('-'));
console.log('at                    :', mk().at(0), mk().at(-1));
// sort/reverse/fill/copyWithin mutate in place and return the receiver.
console.log('reverse               :', [...mk().reverse()].join(','));
console.log('sort is numeric       :', [...Buffer.from([10, 9, 1]).sort()].join(','));
console.log('copyWithin            :', [...mk().copyWithin(0, 1)].join(','));
console.log('fill                  :', [...mk().fill(9)].join(','));
console.log('keys/values/entries   :', [...mk().keys()].join(','), [...mk().values()].join(','), [...mk().entries()].map(e => e.join(':')).join(','));
// The result of a derivation keeps the receiver's own type ("species").
console.log('map keeps Buffer      :', Buffer.isBuffer(mk().map(x => x)));
console.log('filter keeps Buffer   :', Buffer.isBuffer(mk().filter(() => true)));
console.log('i32 map keeps kind    :', new Int32Array([1, 2]).map(x => x).constructor.name);
// A typed array sorts numerically where a plain Array sorts by string.
console.log('u8 sort vs array sort :', [...new Uint8Array([10, 9, 1]).sort()].join(','), [10, 9, 1].sort().join(','));

// ── the decode path a body parser actually walks ────────────────────────────
const jsonBytes = Buffer.from('{"a":1,"b":[2,3],"c":"ü"}', 'utf8');
console.log('toString utf8         :', jsonBytes.toString('utf8'));
console.log('TextDecoder(buffer)   :', new TextDecoder('utf-8').decode(jsonBytes));
console.log('TextDecoder(u8 copy)  :', new TextDecoder('utf-8').decode(new Uint8Array(jsonBytes)));
console.log('parsed                :', JSON.stringify(JSON.parse(jsonBytes.toString('utf8'))));
console.log('stringify buffer      :', JSON.stringify(buf));
