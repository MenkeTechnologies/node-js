// Timers fire in order of their DEADLINE — the time they were scheduled plus
// their delay — not of the delay alone. A chain of 5 ms timers each scheduled
// from the previous one's callback runs at 5, 10, 15, then a 50 ms step lands
// at 65, after the 60 ms timer scheduled at the start. The virtual clock ran
// every 5 ms timer first, however late it was scheduled.
const order = [];
setTimeout(() => order.push('sixty'), 60);
setTimeout(() => {
  order.push('five');
  setTimeout(() => {
    order.push('ten');
    setTimeout(() => {
      order.push('fifteen');
      setTimeout(() => { order.push('sixty-five'); console.log(order.join(' ')); }, 50);
    }, 5);
  }, 5);
}, 5);
setTimeout(() => order.push('thirty'), 30);
