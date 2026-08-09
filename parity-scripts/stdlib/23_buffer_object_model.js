// A Buffer is a real Uint8Array subclass instance: prototype chain, own keys,
// brand checks, and util.inspect rendering.
const b = Buffer.from([1, 2, 3]);
console.log(Object.getPrototypeOf(b) === Buffer.prototype);
console.log(Object.getPrototypeOf(Buffer.prototype) === Uint8Array.prototype);
console.log(typeof Buffer.prototype, b instanceof Buffer, b instanceof Uint8Array);
console.log(Object.prototype.toString.call(b), ArrayBuffer.isView(b), Buffer.isBuffer(b));
console.log(b.constructor.name);

console.log(JSON.stringify(Object.keys(b)));
console.log(JSON.stringify(Object.getOwnPropertyNames(b)));
console.log(JSON.stringify(Object.entries(b)));
console.log(JSON.stringify(Object.values(b)));
console.log(JSON.stringify({ ...b }));
console.log(JSON.stringify([...b]), JSON.stringify(Array.from(b)));
console.log(Object.prototype.hasOwnProperty.call(b, "length"), Object.prototype.hasOwnProperty.call(b, "0"));
console.log(Object.getOwnPropertyDescriptor(b, "length"));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(b, "1")));

console.log(JSON.stringify(b), JSON.stringify(b.toJSON()));
console.log(b, { wrapped: b }, [b]);
console.log(Buffer.alloc(0), Buffer.alloc(51, 1));
console.log(require("util").inspect(Buffer.from("hi")));
console.log(`${Buffer.from("hi")}`, String(Buffer.from("hi")), "" + Buffer.from("hi"));
console.log(Buffer.prototype.toString.call(Buffer.from("abc"), "hex"));
console.log(b.length, b.byteLength, b.byteOffset, b.BYTES_PER_ELEMENT);
