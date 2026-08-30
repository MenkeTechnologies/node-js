// EventEmitter. Nothing in the corpus reached it, and six of its documented
// behaviours were missing outright.
const EventEmitter = require("events");

// Listener order, and `once` firing exactly once.
const e = new EventEmitter();
const seen = [];
e.on("a", (x) => seen.push("on1:" + x));
e.on("a", (x) => seen.push("on2:" + x));
e.once("a", (x) => seen.push("once:" + x));
e.emit("a", 1);
e.emit("a", 2);
console.log("order   ", seen.join(","));
console.log("counts  ", e.listenerCount("a"), e.listeners("a").length, e.eventNames().join(","));
console.log("returns ", e.emit("nobody"), e.emit("a"));

// `prependListener` puts the handler FIRST. It was appending, so it and `on`
// were indistinguishable.
const pre = new EventEmitter();
const order = [];
pre.on("x", () => order.push("second"));
pre.prependListener("x", () => order.push("first"));
pre.prependOnceListener("x", () => order.push("zeroth"));
pre.emit("x");
pre.emit("x");
console.log("prepend ", order.join(","));

// An `error` event with NO listener throws rather than being dropped — this is
// how node surfaces a failed socket, stream or request, and swallowing it
// turned every such failure into silence.
const bare = new EventEmitter();
try { bare.emit("error", new Error("boom")); console.log("unhandled", "NO THROW"); }
catch (err) { console.log("unhandled", err.message); }
// With a listener it is delivered, not thrown, and `emit` reports true.
const held = new EventEmitter();
let got;
held.on("error", (err) => { got = err.message; });
console.log("handled ", held.emit("error", new Error("delivered")), got);

// The `newListener` / `removeListener` meta-events were never emitted.
const meta = new EventEmitter();
const log = [];
meta.on("newListener", (name) => log.push("new:" + name));
meta.on("removeListener", (name) => log.push("rm:" + name));
const handler = () => {};
meta.on("t", handler);
meta.off("t", handler);
// Removing something that was never added emits nothing.
meta.off("t", handler);
console.log("meta    ", log.join(","));

// `setMaxListeners` discarded its argument and `getMaxListeners` always
// answered the default.
const cap = new EventEmitter();
console.log("maxlist ", cap.getMaxListeners(), (cap.setMaxListeners(5), cap.getMaxListeners()));
console.log("static  ", EventEmitter.defaultMaxListeners, typeof EventEmitter.once);

// `rawListeners` was missing entirely.
const raw = new EventEmitter();
const fn = () => {};
raw.once("u", fn);
console.log("raw     ", raw.rawListeners("u").length, raw.listeners("u")[0] === fn);

// The rest of the surface, unchanged by any of the above.
const misc = new EventEmitter();
console.log("chain   ", misc.on("c", () => {}) === misc);
misc.removeAllListeners();
console.log("removeAll", misc.eventNames().length);
class Sub extends EventEmitter {}
const sub = new Sub();
let hits = 0;
sub.on("k", () => hits++);
sub.emit("k");
console.log("subclass", hits, sub instanceof EventEmitter);
