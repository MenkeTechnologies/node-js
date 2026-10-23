// `performance` is the one instance of `Performance`: `instanceof` (it is an
// EventTarget too), its prototype, `constructor`, the brand, and methods that
// are inherited from `Performance.prototype` rather than owned. Every one of
// these disagreed, `process` branded `[object Object]`, and `new
// Performance()` reported "not a constructor" instead of node's
// ERR_ILLEGAL_CONSTRUCTOR.
const t0 = performance.now();
performance.mark('a'); performance.measure('m', 'a');
console.log(typeof t0, performance.getEntriesByName('a').length, performance.getEntriesByType('measure').length > 0);
console.log(performance.constructor === Performance, performance.now === Performance.prototype.now, Object.keys(performance).length, performance.hasOwnProperty('now'), 'now' in performance);
console.log(performance instanceof Performance, performance instanceof EventTarget, Object.getPrototypeOf(performance) === Performance.prototype);
console.log(Object.prototype.toString.call(performance), Object.prototype.toString.call(process));
try { new Performance(); } catch (e) { console.log(e.name, e.code, e.message); }
const { performance: p2, Performance: P2 } = require('perf_hooks');
console.log(p2 === performance, P2 === Performance, typeof p2.timeOrigin);
const f = performance.timerify(function g() { return 7; }); console.log(f());
