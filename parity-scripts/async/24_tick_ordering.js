// process.nextTick vs promise microtasks vs timers.
//
// Deliberately no top-level `setTimeout(0)` vs `setImmediate` race: real Node
// resolves that one nondeterministically (the first loop turn may or may not
// have reached the timer's deadline), so it can never be a parity assertion.
// `setImmediate` is exercised from INSIDE a timer callback, where Node's phase
// order (check phase after the timers phase) makes it deterministic.
console.log("sync-1");
setTimeout(() => {
  console.log("timeout-outer");
  process.nextTick(() => console.log("tick-in-timeout"));
  Promise.resolve().then(() => console.log("micro-in-timeout"));
  setImmediate(() => console.log("immediate-in-timeout"));
  setTimeout(() => console.log("timeout-nested"), 0);
}, 0);
process.nextTick(() => {
  console.log("tick-1");
  process.nextTick(() => console.log("tick-nested"));
});
Promise.resolve()
  .then(() => console.log("micro-1"))
  .then(() => console.log("micro-2"));
queueMicrotask(() => console.log("qm-1"));
process.nextTick(() => console.log("tick-2"));
(async () => {
  console.log("async-sync");
  await null;
  console.log("after-await");
})();
setTimeout(() => console.log("timeout-late"), 5);
console.log("sync-2");
