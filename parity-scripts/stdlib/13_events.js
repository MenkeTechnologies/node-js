// events: on / once / emit / removeListener / listenerCount / emit-order.
const EventEmitter = require("events");

const ee = new EventEmitter();
const log = [];

ee.on("data", (x) => log.push("first:" + x));
ee.on("data", (x) => log.push("second:" + x));
ee.emit("data", 1);
console.log(log.join(","));

ee.once("boot", () => log.push("boot-once"));
ee.emit("boot");
ee.emit("boot");
console.log(log.filter((l) => l.startsWith("boot")).length);

console.log(ee.listenerCount("data"));

const handler = () => log.push("removable");
ee.on("temp", handler);
console.log(ee.listenerCount("temp"));
ee.removeListener("temp", handler);
console.log(ee.listenerCount("temp"));

ee.emit("data", 2);
console.log(log.join(","));

console.log(ee.eventNames().sort().join(","));
