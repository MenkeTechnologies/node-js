// Core-module exports node defines as ACCESSORS (measured:
// CORE_ACCESSOR_EXPORTS) describe themselves as getters — with node's getter
// and setter names and attributes — and util.inspect of the module shows them
// as [Getter] / [Getter/Setter]. They were plain data properties holding the
// value, and `timers` inspected `promises` as the whole promises module.
const util = require('util');
const picks = [
  ['timers', 'promises'], ['dns', 'promises'], ['stream', 'promises'], ['fs', 'promises'],
  ['crypto', 'webcrypto'], ['crypto', 'subtle'], ['crypto', 'getRandomValues'],
  ['util', 'aborted'], ['events', 'captureRejections'], ['events', 'defaultMaxListeners'],
  ['http', 'globalAgent'], ['buffer', 'INSPECT_MAX_BYTES'], ['async_hooks', 'AsyncLocalStorage'],
];
for (const [id, k] of picks) {
  const m = require(id);
  const d = Object.getOwnPropertyDescriptor(m, k);
  console.log(id, k, typeof d.get, d.get && d.get.name, d.get && d.get.length, typeof d.set,
    d.set && d.set.name, d.set && d.set.length, d.enumerable, d.configurable, 'value' in d, d.get.call(m) === m[k]);
}
console.log(util.inspect(require('timers')));
const ee = require('events');
const before = ee.defaultMaxListeners;
Object.getOwnPropertyDescriptor(ee, 'defaultMaxListeners').set.call(ee, 7);
console.log(before, ee.defaultMaxListeners);
ee.defaultMaxListeners = before;
// http.globalAgent and https.globalAgent are one object each; every read built a new one.
console.log(require('http').globalAgent === require('http').globalAgent, require('https').globalAgent === require('https').globalAgent);
