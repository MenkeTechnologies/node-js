// `stream`. Nothing in the corpus reached it. Three things were wrong, and the
// first two are the kind that leave a program looking like it works.
const { Readable, Writable, Duplex, Transform, PassThrough } = require("stream");

// `new Transform({ transform(chunk, enc, cb) {…} })` supplies the conversion
// the stream exists to perform, and the option was DISCARDED — so a Transform
// passed every chunk through unchanged while still emitting and ending
// normally. Nothing about the stream's behaviour said it had been ignored.
const upper = new Transform({ transform(c, e, cb) { cb(null, String(c).toUpperCase()); } });
const upperOut = [];
upper.on("data", (d) => upperOut.push(String(d)));
upper.write("a"); upper.write("b"); upper.end("c");
console.log("transform", upperOut.join(","));
// The callback decides the output, so a nullish chunk contributes nothing and
// the raw chunk is never emitted alongside it.
const drop = new Transform({ transform(c, e, cb) { cb(null, String(c) === "skip" ? null : c); } });
const dropOut = [];
drop.on("data", (d) => dropOut.push(String(d)));
drop.write("keep"); drop.write("skip"); drop.end("last");
console.log("filter   ", dropOut.join(","));
// A `write` implementation is a sink: it sees the chunk and the chunk still
// flows on unchanged.
const sunk = [];
const sink = new Writable({ write(c, e, cb) { sunk.push("w:" + c); cb(); } });
sink.write("1"); sink.end("2");
console.log("writable ", sunk.join(","));

// `readable`/`writable`/`destroyed` are live properties and all read back as
// undefined. Only the side that APPLIES gets one — node leaves `writable`
// undefined on a plain Readable rather than reporting false.
const r = new Readable({ read() {} });
const w = new Writable({ write(c, e, cb) { cb(); } });
const pt = new PassThrough();
console.log("sides-r  ", r.readable, r.writable, r.destroyed);
console.log("sides-w  ", w.readable, w.writable, w.destroyed);
console.log("sides-pt ", pt.readable, pt.writable);
// The state changes BEFORE the listeners run, so a handler asking about the
// stream it is being told about gets the new answer. Recorded rather than
// printed from the handler: this model emits `finish` synchronously from
// `end()` where node defers it, so the two interleave differently and only the
// VALUE is the same.
let finishSaw;
w.on("finish", () => { finishSaw = w.writable; });
w.end();
console.log("after-end", w.writable);

// `Readable.from` did not exist. Its whole point is that the caller attaches a
// `data` listener AFTER the call returns, so the items are queued and drained
// on a later tick rather than emitted while the stream is being built.
const seen = [];
Readable.from([1, 2]).on("data", (x) => seen.push("arr:" + x)).on("end", () => {
  // A string is ONE chunk, not one per character.
  Readable.from("ab").on("data", (x) => seen.push("str:" + x)).on("end", () => {
    Readable.from(new Set(["s"])).on("data", (x) => seen.push("set:" + x)).on("end", () => {
      Readable.from((function* () { yield "g1"; yield "g2"; })())
        .on("data", (x) => seen.push("gen:" + x))
        .on("end", () => {
          Readable.from([]).on("data", () => seen.push("never")).on("end", () => {
            console.log("from     ", seen.join(","));
            console.log("statics  ", typeof Readable.from, typeof Duplex.from, typeof Readable.isDisturbed);
            // By now `finish` has fired on both engines, so what the handler
            // saw can be compared: `writable` was already false inside it.
            console.log("in-finish", finishSaw);
            const done = Readable.from(["x"]);
            done.on("data", () => {}).on("end", () => console.log("ended    ", done.readable));
          });
        });
    });
  });
});

// `require('stream')` IS the `Stream` constructor, not a namespace beside one.
// It used to resolve to a plain namespace object, so `typeof` was `object`,
// `stream.Stream === stream` was false, `.prototype` was `undefined`, and the
// ES5 subclassing pattern libraries still ship threw outright.
const S = require("stream");
const EE = require("events");
console.log("ctor    ", typeof S, S.Stream === S, S.name, typeof S.prototype);
// The hierarchy is real: Readable → Stream → EventEmitter → Object. Every
// prototype used to hang straight off Object.prototype, so `instanceof` only
// worked through a native-tag special case one link deep.
console.log("chain   ", Object.getPrototypeOf(S.Readable.prototype) === S.prototype,
  Object.getPrototypeOf(S.prototype) === EE.prototype,
  Object.getPrototypeOf(S.Writable.prototype) === S.prototype);
const rd = new S.Readable({ read() {} });
console.log("instance", rd instanceof S.Readable, rd instanceof S, rd instanceof EE,
  Object.getPrototypeOf(rd) === S.Readable.prototype);
console.log("classes ", ["Readable", "Writable", "Duplex", "Transform", "PassThrough", "Stream"]
  .map((k) => typeof S[k]).join(","));
console.log("members ", typeof S.promises, typeof S.pipeline, typeof S.finished,
  ["Readable", "Writable", "Stream"].every((k) => Object.keys(S).includes(k)));
// The ES5 pattern: borrow the constructor, then adopt its prototype.
function Legacy() { S.call(this); }
Legacy.prototype = Object.create(S.prototype);
Legacy.prototype.constructor = Legacy;
const legacy = new Legacy();
console.log("es5     ", legacy instanceof S, legacy instanceof Legacy, typeof legacy.pipe, typeof legacy.on);

// Two chain roots that were wrong for every object, not just streams:
// `Object.prototype` reported ITSELF as its prototype — an endless chain to
// anything walking one — and a builtin prototype namespace reported a handle
// for its own constructor rather than `Object.prototype`.
console.log("roots   ", Object.getPrototypeOf(Object.prototype),
  Object.getPrototypeOf(Array.prototype) === Object.prototype,
  Object.getPrototypeOf(Function.prototype) === Object.prototype);
// A walk from any ordinary object now terminates.
let node = Object.getPrototypeOf({}), hops = 0;
while (node !== null && hops < 10) { node = Object.getPrototypeOf(node); hops++; }
console.log("walk    ", hops, node);
