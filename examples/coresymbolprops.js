// A core function's own symbol-keyed properties, as node defines them
// (measured: CORE_SYMBOL_PROPS): os.freemem's enumerable Symbol.toPrimitive,
// setTimeout's enumerable util.promisify.custom GETTER, fs.exists' frozen one.
// util.inspect shows the enumerable ones, getOwnPropertySymbols lists them,
// the descriptors carry node's attributes, and propertyIsEnumerable answers
// for a symbol key — it read false for every symbol-keyed property.
const util = require('util'), os = require('os');
const cases = [
  ['os.freemem', os.freemem],
  ['setTimeout', setTimeout],
  ['setImmediate', setImmediate],
  ['timers.setTimeout', require('timers').setTimeout],
  ['stream.pipeline', require('stream').pipeline],
  ['fs.exists', require('fs').exists],
  ['child_process.exec', require('child_process').exec],
];
for (const [name, f] of cases) {
  console.log(name, util.inspect(f));
  for (const s of Object.getOwnPropertySymbols(f)) {
    const d = Object.getOwnPropertyDescriptor(f, s);
    const fn = d.get || d.value;
    console.log(' ', String(s), d.get ? 'get' : 'value', fn.name, fn.length, d.enumerable, d.configurable,
      'writable' in d ? d.writable : '-', f.propertyIsEnumerable(s), d.get ? d.get.call(f) === f[s] : typeof d.value);
  }
}
console.log(util.inspect({ t: setTimeout, m: os.totalmem }));
const s = Symbol('s');
const o = { [s]: 1, a: 2 };
Object.defineProperty(o, Symbol.iterator, { value: 1, enumerable: false });
console.log(o.propertyIsEnumerable(s), o.propertyIsEnumerable(Symbol.iterator), o.propertyIsEnumerable('a'), o.propertyIsEnumerable(Symbol('x')));
function f() {} f[s] = 1;
const a = [1]; a[s] = 2;
console.log(f.propertyIsEnumerable(s), a.propertyIsEnumerable(s), a.propertyIsEnumerable(0), a.propertyIsEnumerable('length'));
