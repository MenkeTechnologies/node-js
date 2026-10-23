// A core-module function written as an ordinary `function` owns a writable,
// non-enumerable, non-configurable `prototype` whose `constructor` is the
// function (measured per function into `CORE_PROTOTYPED`); an arrow or a
// method owns none. No builtin function had one. Also: the three exports the
// arity table missed — `crypto.getRandomValues` and `util.aborted` are
// accessors in node, and `assert.strict` is keyed differently here.
const fs = require('fs'), path = require('path'), util = require('util');
for (const f of [fs.readFileSync, path.join, util.format, fs.promises.readFile, require('events').once, require('os').cpus]) {
  const d = Object.getOwnPropertyDescriptor(f, 'prototype');
  console.log(f.name, Object.getOwnPropertyNames(f).join(), 'prototype' in f, f.hasOwnProperty('prototype'), d && [d.writable, d.enumerable, d.configurable].join(), d && f.prototype === f.prototype, d && f.prototype.constructor === f, d && Object.getOwnPropertyNames(f.prototype).join(), Object.keys(f).length);
}
const r = fs.readFileSync; r.prototype.x = 1; console.log(fs.readFileSync.prototype.x);
console.log(require('util').aborted.length, require('crypto').getRandomValues.length, require('assert').strict.name, require('assert').strict.length);
