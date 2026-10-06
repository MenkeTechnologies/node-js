// DataView renders its [byteLength]/[byteOffset]/[buffer] view fields, and
// subclasses of DataView, typed arrays and ArrayBuffer carry the
// `Ctor(n) [Tag]` prefix and `[Ctor [Tag]]` depth stub of node's getPrefix.
const b=new ArrayBuffer(4); const d=new DataView(b,1,2); d.x=5;
console.log(d);
console.log({a:{b:{c:new DataView(b)}}});
console.log({a:{b:new DataView(b)}});
class DV extends DataView{}; console.log(new DV(b));
console.log(require("util").inspect(new DataView(b),{showHidden:true}));
console.log([new DataView(new ArrayBuffer(0))]);
class U extends Uint8Array{}; console.log(new U(2), {a:{b:{c:new U(1)}}});
class A extends ArrayBuffer{}; console.log(new A(1), {a:{b:{c:new A(1)}}});
console.log({a:{b:{c:new DV(b)}}});
console.log({a:{b:{c:new Uint8Array(1)}}});
