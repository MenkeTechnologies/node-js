// The WHATWG stream, messaging and performance classes are globals as well as
// module members (the same constructors); a ReadableStream is async-iterable;
// the nested-module cross-links are enumerable and identical to the modules;
// util.promisify honours custom forms; process warnings arrive on the next
// tick with a `warning` event; and EventEmitter carries its statics.
const web = require("stream/web"), perf = require("perf_hooks"), wt = require("worker_threads");
const util = require("util"), EE = require("events");
const t = (label, f) => {
  try {
    console.log(label, JSON.stringify(f()));
  } catch (e) {
    console.log(label, e.name, e.code, e.message);
  }
};

t("globals   ", () => ["ReadableStream", "WritableStream", "TransformStream", "TextDecoderStream", "CompressionStream", "CountQueuingStrategy"].map((n) => typeof globalThis[n] === "function" && globalThis[n] === web[n]));
t("perfGlob  ", () => ["PerformanceObserver", "PerformanceEntry", "PerformanceMark", "Performance"].map((n) => globalThis[n] === perf[n]));
t("msgGlob   ", () => [MessageChannel === wt.MessageChannel, BroadcastChannel === wt.BroadcastChannel]);
t("notEnum   ", () => Object.keys(globalThis).includes("ReadableStream"));

// Cross-links: the same object as the module, and listed by Object.keys.
t("sameMods  ", () => [
  require("timers").promises === require("timers/promises"),
  require("dns").promises === require("dns/promises"),
  require("crypto").subtle === globalThis.crypto.subtle,
]);
t("enumMods  ", () => ["timers", "dns", "stream", "readline", "assert", "path", "crypto"].map((m) => Object.keys(require(m)).filter((k) => ["promises", "strict", "posix", "win32", "webcrypto", "subtle"].includes(k)).sort().join("+")));
t("dnsKeys   ", () => Object.keys(require("dns")).some((k) => k.startsWith("promise") && k !== "promises"));

// EventEmitter statics, and the error monitor.
t("eeStatics ", () => [EE.captureRejections, EE.usingDomains, EE.captureRejectionSymbol === Symbol.for("nodejs.rejection"), typeof EE.errorMonitor, Symbol.keyFor(EE.errorMonitor), String(EE.errorMonitor)]);
const em = new EE(), seen = [];
em.on(EE.errorMonitor, (e) => seen.push(`monitor:${e.message}`));
t("monitorOnly", () => em.emit("error", new Error("x")));
em.on("error", (e) => seen.push(`handler:${e.message}`));
em.emit("error", new Error("y"));
t("monitored ", () => seen);

// util.promisify.
function foo(a, b, c) {}
const p = util.promisify(foo);
t("promisify ", () => [p.name, p.length, util.promisify(p) === p, util.promisify(setTimeout) === require("timers/promises").setTimeout]);
function custom(cb) { cb(null, 1); }
custom[util.promisify.custom] = () => "custom";
t("custom    ", () => util.promisify(custom)());
const bad = () => {};
bad[util.promisify.custom] = 5;
t("customBad ", () => util.promisify(bad));

// styleText: outermost first, inner resets re-open, hex colors, and no codes
// at all when the stream (here a pipe) would not colorize.
const o = { validateStream: false };
t("style     ", () => [util.styleText(["bold", "red"], "x", o), util.styleText("red", util.styleText("blue", "in", o) + "out", o), util.styleText(["bold", "dim"], "a\x1b[22mb", o)]);
t("styleHex  ", () => [util.styleText("#ff8000", "x", o), util.styleText("#0f0", "x", o)]);
t("styleBad  ", () => util.styleText("#ggg", "x", o));
t("styleText ", () => util.styleText("red", 5, o));
t("styleStrm ", () => util.styleText("red", "x", { stream: 5 }));

process.on("warning", (w) => console.log("warning   ", w.name, w.message.slice(0, 44), w.code));
process.emitWarning("first", { code: "C1" });
process.emitWarning(new Error("boom"));
const lim = new EE();
lim.setMaxListeners(1);
lim.on("a", () => {});
lim.on("a", () => {});
lim.on("a", () => {});
console.log("sync      done");

(async () => {
  const log = [];
  const rs = new ReadableStream({
    start(c) { c.enqueue("a"); c.enqueue("b"); c.enqueue("c"); c.close(); },
    cancel(r) { log.push(`cancel:${r}`); },
  });
  for await (const ch of rs) {
    log.push(ch);
    if (ch === "b") break;
  }
  console.log("forAwait  ", log.join(), rs.locked);
  const keep = new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close(); } });
  for await (const ch of keep.values({ preventCancel: true })) { log.push(ch); break; }
  console.log("prevent   ", JSON.stringify(await keep.getReader().read()));
  const locked = new ReadableStream();
  locked.getReader();
  t("locked    ", () => locked.values());
  const ts = new TransformStream({ transform(ch, c) { c.enqueue(ch.toUpperCase()); } });
  const w = ts.writable.getWriter();
  w.write("x");
  w.write("y");
  w.close();
  const up = [];
  for await (const ch of ts.readable) up.push(ch);
  console.log("transform ", up.join());
  const te = new TextEncoderStream(), tw = te.writable.getWriter();
  tw.write("hé");
  tw.close();
  for await (const ch of te.readable) console.log("encoded   ", ch.constructor.name, Array.from(ch).join());
})();
