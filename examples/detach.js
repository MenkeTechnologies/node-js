// An `ArrayBuffer` could not be DETACHED: `detached` and `transfer` did not
// exist, and `structuredClone`'s `transfer` option was ignored, so a buffer
// handed away stayed fully usable where node leaves it empty.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const buffer = new ArrayBuffer(8);
const view = new Uint8Array(buffer);
view[0] = 7;
console.log("before    ", buffer.byteLength, buffer.detached, view.length, view[0]);
const moved = buffer.transfer();
console.log("after     ", buffer.byteLength, buffer.detached, moved.byteLength,
  new Uint8Array(moved)[0]);
// The bytes move; the source keeps nothing.
console.log("resizable ", new ArrayBuffer(4).resizable, new ArrayBuffer(4).maxByteLength,
  new ArrayBuffer(4, { maxByteLength: 8 }).resizable);
show("grow      ", () => { const b = new ArrayBuffer(4); const g = b.transfer(8); return [g.byteLength, b.detached]; });
show("shrink    ", () => { const b = new ArrayBuffer(4); const s = b.transferToFixedLength(2); return [s.byteLength, s.resizable]; });
show("retransfer", () => buffer.transfer());
show("bufferSlice", () => buffer.slice(0).byteLength);
show("construct ", () => { new Uint8Array(buffer); return "ok"; });

// A view over a detached buffer reports ZERO extent and reads `undefined` —
// its own `length` still holds the old count, so reading that back made a
// detached view still look eight bytes long.
console.log("view      ", view.length, view.byteLength, view[0], view.buffer === buffer);
// Its own index properties are gone with the bytes, so it enumerates as empty,
// `in` and `hasOwnProperty` answer false, and a write is DROPPED rather than
// stored as an ordinary property.
console.log("enumerate ", Object.keys(view).length, JSON.stringify(Object.keys(view)),
  Object.getOwnPropertyNames(view).length);
console.log("membership", 0 in view, view.hasOwnProperty(0),
  JSON.stringify(Object.getOwnPropertyDescriptor(view, "0")));
view[0] = 5;
console.log("write     ", view[0], JSON.stringify(Object.getOwnPropertyDescriptor(view, "0")));
console.log("inspect   ", require("util").inspect(view), JSON.stringify(view));
// …but every METHOD over it throws, naming itself. Reads answering zero while
// methods throw is why the guard is per-method and not inside the accessors.
show("spread    ", () => [...view]);
show("join      ", () => view.join(","));
show("slice     ", () => [...view.slice(0)]);
show("forEach   ", () => { view.forEach(() => {}); return "ok"; });
show("indexOf   ", () => view.indexOf(7));
// A DataView THROWS where a typed array answers zero, and names the getter.
show("dataview  ", () => { const b = new ArrayBuffer(4); const d = new DataView(b); b.transfer(); return d.getInt8(0); });
show("dv-extent ", () => { const b = new ArrayBuffer(4); const d = new DataView(b); b.transfer(); return d.byteLength; });

// `structuredClone`'s `transfer` list detaches each buffer once the clone has
// its bytes, and rejects anything that is not an ArrayBuffer.
show("sc-transfer", () => { const b = new ArrayBuffer(8); const c = structuredClone(b, { transfer: [b] }); return [c.byteLength, b.byteLength, b.detached]; });
show("sc-bytes  ", () => { const b = new ArrayBuffer(4); new Uint8Array(b)[0] = 9; const c = structuredClone({ b }, { transfer: [b] }); return [new Uint8Array(c.b)[0], b.detached]; });
// A buffer in the list need not appear in the graph at all.
show("sc-outside", () => { const b = new ArrayBuffer(2); const c = structuredClone({}, { transfer: [b] }); return [JSON.stringify(c), b.detached]; });
show("sc-badlist", () => structuredClone({}, { transfer: [{}] }));
show("sc-detached", () => { const b = new ArrayBuffer(8); b.transfer(); return structuredClone(b); });
show("sc-empty  ", () => structuredClone({ a: 1 }, { transfer: [] }).a);
show("sc-noopts ", () => [structuredClone({ a: 1 }, {}).a, structuredClone({ a: 1 }, undefined).a]);
