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
