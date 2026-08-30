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
