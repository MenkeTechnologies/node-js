// process.nextTick vs promise microtasks vs timers.
//
// Deliberately no top-level `setTimeout(0)` vs `setImmediate` race: real Node
// resolves that one nondeterministically (the first loop turn may or may not
// have reached the timer's deadline), so it can never be a parity assertion.
// `setImmediate` is exercised from INSIDE a timer callback, where Node's phase
// order (check phase after the timers phase) makes it deterministic.
//
// The late timer's delay is 100ms and not 5ms for the SAME reason. At 5ms its
// deadline can fall inside the outer timer's callback chain, and then whether
// `timeout-late` lands before `immediate-in-timeout`, between it and
// `timeout-nested`, or after both is a wall-clock race in the ORACLE: measured
// on node v26.7.0, 60 runs of the 5ms version produced three different
// orderings (52 / 4 / 4). Anything shorter than the work it has to outlast is
// not a parity assertion, it is a coin flip.
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
setTimeout(() => console.log("timeout-late"), 100);
console.log("sync-2");
