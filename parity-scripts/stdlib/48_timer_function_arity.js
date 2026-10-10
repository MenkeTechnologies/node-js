// name/length of the timer functions, which no ECMAScript table covers.
const timers = require("timers");
const tp = require("timers/promises");
for (const [label, f] of [
  ["setTimeout", setTimeout], ["setInterval", setInterval], ["setImmediate", setImmediate],
  ["clearTimeout", clearTimeout], ["clearInterval", clearInterval], ["clearImmediate", clearImmediate],
  ["nextTick", process.nextTick], ["queueMicrotask", queueMicrotask],
  ["timers.setTimeout", timers.setTimeout], ["timers.setInterval", timers.setInterval],
  ["timers.setImmediate", timers.setImmediate], ["timers.clearTimeout", timers.clearTimeout],
  ["promises.setTimeout", tp.setTimeout], ["promises.setImmediate", tp.setImmediate], ["promises.setInterval", tp.setInterval],
]) console.log(label, f.name, f.length);
