// EventEmitter `once` registrations are node's `_onceWrap` wrappers, and
// `removeListener` / `removeAllListeners` follow `lib/events.js`. The
// listener used to be stored unwrapped, so `rawListeners` could not tell a
// once-listener from an `on` one and `removeListener` matched the wrong entry.
const EventEmitter = require("events");

function f() {}
function a() {}
function b() {}

// `rawListeners` hands back the wrapper; `listeners` unwraps it.
const e = new EventEmitter();
e.once("x", f);
const raw = e.rawListeners("x")[0];
console.log("raw     ", raw === f, raw.listener === f, raw.name, raw.length);
console.log("unwrap  ", e.listeners("x")[0] === f);

// `removeListener` takes the LAST match, so the `on` registration added after
// the `once` goes first and the wrapper stays.
e.on("x", f);
e.removeListener("x", f);
console.log("lastwins", e.listenerCount("x"), e.rawListeners("x")[0] === raw);

// Calling the wrapper by hand fires once and removes it.
const e2 = new EventEmitter();
let n = 0;
e2.once("z", () => n++);
const w = e2.rawListeners("z")[0];
w();
w();
e2.emit("z");
console.log("manual  ", n, e2.listenerCount("z"));

// `listenerCount(type, listener)` counts a wrapper for the function it wraps.
const e3 = new EventEmitter();
e3.once("x", a);
e3.on("x", b);
e3.once("x", a);
console.log("count   ", e3.listenerCount("x"), e3.listenerCount("x", a), e3.listenerCount("x", b));
e3.removeListener("x", a);
console.log("removed ", e3.rawListeners("x").map((l) => l.name).join(","), e3.listeners("x").map((l) => l.name).join(","));
console.log("statics ", EventEmitter.getEventListeners(e3, "x").map((l) => l.name).join(","));

// The `removeListener` event: a lone listener reports `list.listener`, a
// longer list reports exactly what was passed — the wrapper, when a
// once-listener removes itself after firing.
const lone = new EventEmitter();
lone.on("removeListener", (t, l) => console.log("lone    ", t, l === f, l.name));
lone.once("k", f);
lone.emit("k");
const many = new EventEmitter();
many.on("removeListener", (t, l) => console.log("many    ", t, l === f, l.name));
many.on("k", a);
many.once("k", f);
many.emit("k");

// `removeAllListeners` announces every removal, LIFO within an event, every
// other event before `removeListener` itself.
const log = [];
const all = new EventEmitter();
all.on("removeListener", (t, l) => log.push(String(t) + ":" + l.name));
all.on("x", a);
all.once("x", b);
all.on("y", a);
all.removeAllListeners("x");
console.log("rmall x ", log.join(" "));
log.length = 0;
all.on("x", a);
all.removeAllListeners();
console.log("rmall   ", log.join(" "), all.eventNames().length);

// A once-listener removed by an earlier listener of the same emit still runs
// in that emit (the list is copied on write), and never again.
const order = [];
const e4 = new EventEmitter();
function second() {
  order.push("second");
}
e4.on("q", function first() {
  order.push("first");
  e4.removeListener("q", second);
});
e4.once("q", second);
e4.emit("q");
e4.emit("q");
console.log("order   ", order.join(","));

// `newListener` is handed the function registered, not the wrapper.
const e5 = new EventEmitter();
e5.on("newListener", (t, l) => console.log("newlis  ", t, l === f));
e5.once("p", f);
