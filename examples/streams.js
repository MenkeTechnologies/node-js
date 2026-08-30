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
