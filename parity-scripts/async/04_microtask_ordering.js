// Deterministic event-loop ordering: sync -> nextTick -> promise -> timeout.
const order = [];
order.push('sync-start');

setTimeout(() => {
  order.push('timeout');
  console.log(JSON.stringify(order));
}, 0);

Promise.resolve().then(() => order.push('promise'));

process.nextTick(() => order.push('nextTick'));

order.push('sync-end');
