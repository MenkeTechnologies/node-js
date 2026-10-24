// %AsyncIteratorPrototype%, %AsyncGeneratorPrototype% and %GeneratorPrototype%
// as real objects on the generator chain. node's own lib/ reaches the first
// through the idiom below; it read null.
const AGF = Object.getPrototypeOf(async function* () {});
const GF = Object.getPrototypeOf(function* () {});
const AGP = AGF.prototype;
const GP = GF.prototype;
const AIP = Object.getPrototypeOf(AGP);

console.log('agp', typeof AGP, Object.getOwnPropertyNames(AGP), Object.getOwnPropertySymbols(AGP));
console.log('aip', AIP !== Object.prototype, Object.getOwnPropertySymbols(AIP));
console.log('aip proto', Object.getPrototypeOf(AIP) === Object.prototype);
console.log('gp', Object.getOwnPropertyNames(GP), Object.prototype.toString.call(GP), Object.prototype.toString.call(AGP));
console.log('ctor', GP.constructor === GF, AGP.constructor === AGF);
const d = Object.getOwnPropertyDescriptor(GF, 'prototype');
console.log('desc', d.writable, d.enumerable, d.configurable, d.value === GP);
for (const m of ['next', 'return', 'throw']) {
  console.log('name', GP[m].name, GP[m].length, AGP[m].name, AGP[m].length);
  try { GP[m].call({}); } catch (e) { console.log('sync', e.constructor.name, e.message); }
}
console.log('fn names', AIP[Symbol.asyncIterator].name, AIP[Symbol.asyncDispose].name);
console.log('self', AIP[Symbol.asyncIterator].call(5));

function* f() { yield 1; yield 2; }
f.prototype.count = function () { return [...this].length; };
console.log('own proto', Object.getPrototypeOf(f.prototype) === GP, Object.getOwnPropertyNames(f.prototype));
console.log('instance', Object.getPrototypeOf(f()) === f.prototype, f() instanceof f, f().count());
f.prototype = 3;
console.log('fallback', Object.getPrototypeOf(f()) === GP);

async function* af() { yield 'a'; }
const g = af();
console.log('async instance', Object.getPrototypeOf(g) === af.prototype, Object.getPrototypeOf(af.prototype) === AGP);
console.log('identity', g.next === AGP.next, g[Symbol.asyncIterator] === AIP[Symbol.asyncIterator]);
console.log('inspect', require('util').inspect(g));

(async () => {
  for (const m of ['next', 'return', 'throw']) {
    await AGP[m].call({}).catch((e) => console.log('async', e.constructor.name, e.message));
  }
  let returned = 0;
  const it = { __proto__: AIP, return() { returned++; return { done: true }; } };
  console.log('iter self', it[Symbol.asyncIterator]() === it);
  console.log('dispose', await it[Symbol.asyncDispose](), returned);
  console.log('no return', await ({ __proto__: AIP })[Symbol.asyncDispose]());
  for await (const v of g) console.log('v', v);
})();
