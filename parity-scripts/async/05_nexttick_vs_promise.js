// nextTick queue fully drains before the promise microtask queue, and
// nested nextTicks scheduled during draining run before promises.
const order = [];

Promise.resolve().then(() => order.push('promise-1'));
process.nextTick(() => {
  order.push('nextTick-1');
  process.nextTick(() => order.push('nextTick-2'));
});
Promise.resolve().then(() => order.push('promise-2'));

setTimeout(() => {
  console.log(JSON.stringify(order));
}, 0);
