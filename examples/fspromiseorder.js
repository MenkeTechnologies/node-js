// A `fs/promises` operation settles when its work completes on the loop (the
// poll phase), never inside the call: the promise jobs and ticks the calling
// code queued afterwards all run first. node-js settled it synchronously, so
// its reaction overtook a microtask queued after it.
//
// Only that much is pinned. Between two pending operations (a promise one and
// a callback one) node's order is a thread-pool race.
const fsp = require("fs/promises");
const log = [];
fsp.readFile(__filename).then(() => log.push("readFile"));
fsp.stat("/nonexistent-path").catch((e) => log.push("stat " + e.code));
Promise.resolve().then(() => log.push("micro"));
process.nextTick(() => log.push("tick"));
(async () => {
  await fsp.access(__filename);
  log.push("await");
})();
queueMicrotask(() => log.push("micro2"));
setTimeout(() => console.log(log.slice(0, 3).join(" "), "|", log.slice(3).sort().join(" ")), 50);
