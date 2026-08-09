// The EventEmitter surface every emitter-backed native instance must carry.
//
// Each dispatcher used to re-list these names by hand, and the copies drifted:
// a socket, an http request/response and a stream were each missing
// `listeners`, `setMaxListeners` and `getMaxListeners`, which is what `unpipe`
// (reached from `body-parser` on every `express.json()` request) calls. The
// probe walks the SAME name list against several different receiver kinds, so a
// dispatcher that re-lists its own subset diverges on whichever names it left
// out — no per-receiver rule can paper over it.
const net = require("net");
const http = require("http");
const { Readable } = require("stream");
const { EventEmitter } = require("events");

const NAMES = [
  "on",
  "addListener",
  "prependListener",
  "once",
  "prependOnceListener",
  "emit",
  "removeListener",
  "off",
  "removeAllListeners",
  "listenerCount",
  "listeners",
  "eventNames",
  "setMaxListeners",
  "getMaxListeners",
];

function surface(label, obj) {
  console.log(label, NAMES.map((n) => (typeof obj[n] === "function" ? 1 : 0)).join(""));
}

surface("emitter ", new EventEmitter());
surface("socket  ", new net.Socket());
surface("server  ", http.createServer());
surface("readable", new Readable({ read() {} }));

// `listeners` must return the actual registered functions, in order, and must
// not be confused with `listenerCount`.
const r = new Readable({ read() {} });
const a = () => {};
const b = () => {};
r.on("data", a);
r.on("data", b);
console.log(r.listeners("data").length, r.listeners("data")[0] === a, r.listeners("data")[1] === b);
console.log(r.listenerCount("data"), r.listeners("nope").length);
r.removeListener("data", a);
console.log(r.listeners("data").length, r.listeners("data")[0] === b);

// `setMaxListeners` is chainable; `getMaxListeners` reports a number.
const s = new net.Socket();
console.log(s.setMaxListeners(5) === s, typeof s.getMaxListeners());
