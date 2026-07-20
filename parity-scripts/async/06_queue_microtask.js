// queueMicrotask ordering relative to promise microtasks and nextTick.
const order = [];
order.push('start');

queueMicrotask(() => order.push('microtask-1'));
Promise.resolve().then(() => order.push('promise'));
queueMicrotask(() => order.push('microtask-2'));
process.nextTick(() => order.push('nextTick'));

order.push('end');

setTimeout(() => console.log(JSON.stringify(order)), 0);
