// Promises, async/await, and event-loop ordering (microtasks vs timers).
function delayValue(v) {
  return new Promise((resolve) => resolve(v));
}

async function compute(a, b) {
  const x = await delayValue(a);
  const y = await delayValue(b);
  return x + y;
}

console.log("start");

compute(3, 4).then((sum) => console.log("compute result", sum));

Promise.resolve(10)
  .then((v) => v * 2)
  .then((v) => console.log("chain", v));

Promise.all([delayValue(1), delayValue(2), 3]).then((arr) =>
  console.log("all", arr)
);

Promise.allSettled([Promise.resolve("ok"), Promise.reject("no")]).then((rs) =>
  console.log("allSettled", rs.map((r) => r.status))
);

async function withCatch() {
  try {
    await Promise.reject(new Error("boom"));
  } catch (e) {
    return "caught: " + e.message;
  }
}
withCatch().then((v) => console.log(v));

process.nextTick(() => console.log("nextTick"));
Promise.resolve().then(() => console.log("microtask"));
setTimeout(() => console.log("timeout"), 0);

console.log("end");

// `Promise.prototype.finally` (27.2.5.3). It is specified as
// `PromiseResolve(onFinally()).then(() => value)`, and neither half was
// happening: the callback's return value was discarded, so a promise it
// returned was never awaited, and the chain settled three microtask ticks
// early. The awaiting is the part real code depends on —
// `work().finally(() => close()).then(next)` ran `next` before `close()` had
// finished.
const fin = [];
const note = (n) => fin.push(n);
Promise.resolve("v")
  .finally(() => new Promise((r) => setTimeout(() => { note("cleanup-done"); r(); }, 10)))
  .then((v) => note("after:" + v));
// Three ticks, not one: `F-then` lands after `a3`, not after `a1`.
Promise.resolve().finally(() => note("F")).then(() => note("F-then"));
Promise.resolve().then(() => note("a1")).then(() => note("a2")).then(() => note("a3")).then(() => note("a4"));
// The callback's own rejection wins over the value being carried.
Promise.resolve("x").finally(() => Promise.reject(new Error("cbfail"))).then(
  (v) => note("kept:" + v),
  (e) => note("overridden:" + e.message),
);
// A non-callable onFinally is handed to `then`, which ignores it.
Promise.resolve("p").finally(null).then((v) => note("nullcb:" + v));
Promise.reject(new Error("q")).finally(undefined).catch((e) => note("undefcb:" + e.message));
// An ordinary return value is discarded; a synchronous throw is not.
Promise.resolve("keep").finally(() => "discard").then((v) => note("kept:" + v));
Promise.resolve("s").finally(() => { throw new Error("sync"); }).catch((e) => note("syncthrow:" + e.message));
setTimeout(() => console.log(fin.join("\n")), 50);

{
// `processTicksAndRejections` runs in ROUNDS: drain the nextTick queue, then
// drain the microtask queue IN FULL, then repeat if those microtasks queued
// more ticks. Preferring ticks on every step interleaved the two, so a tick
// scheduled from inside a `.then` jumped ahead of the promise callbacks already
// queued behind it.
const order = [];
Promise.resolve().then(() => { order.push("p1"); process.nextTick(() => order.push("tick-in-p1")); });
Promise.resolve().then(() => order.push("p2"));
Promise.resolve().then(() => order.push("p3"));
setTimeout(() => console.log("rounds  ", order.join(" ")), 15);

// The full interleaving, including an await resumption and a tick nested in a
// tick. Ticks still win the FIRST round; only ones queued mid-round wait.
const full = [];
process.nextTick(() => full.push("tick1"));
queueMicrotask(() => full.push("micro1"));
Promise.resolve().then(() => full.push("promise1"));
setImmediate(() => full.push("immediate1"));
setTimeout(() => full.push("timeout0"), 0);
process.nextTick(() => { full.push("tick2"); process.nextTick(() => full.push("tick-nested")); });
Promise.resolve().then(() => { full.push("promise2"); process.nextTick(() => full.push("tick-from-promise")); });
(async () => { full.push("async-sync"); await null; full.push("async-after-await"); })();
full.push("sync");
// The tail is deliberately trimmed: whether a 0ms timeout or a setImmediate
// runs first depends on how far into the loop turn the queue already is, and
// node itself is not stable on it once other work is pending.
setTimeout(() => console.log("full    ",
  full.slice(0, full.indexOf("tick-from-promise") + 1).join(" ")), 25);

// A Promise and a generator are ORDINARY objects to `JSON.stringify`: their
// state is internal slots, so they contribute no entries and render as `{}`.
// They were omitted entirely, so one in an array became `null` and one in an
// object vanished.
console.log("json    ", JSON.stringify(Promise.resolve(1)),
  JSON.stringify((function* () {})()), JSON.stringify([Promise.resolve(1)]),
  JSON.stringify({ p: Promise.resolve(1) }));

// A `Timeout` coerces to its own id, which is how a handle stored as
// `Number(t)` can be passed back to `clearTimeout`. The symbol form was missing.
const handle = setTimeout(() => {}, 1000);
console.log("timer   ", typeof handle[Symbol.toPrimitive],
  Number(handle) === handle[Symbol.toPrimitive](), Number.isFinite(Number(handle)),
  handle.valueOf() === handle);
clearTimeout(handle);
}
