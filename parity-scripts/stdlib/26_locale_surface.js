// The `toLocale*` surface.
//
// node-js reads no LANG/LC_ALL/TZ and hardwires these to en-US at UTC, so its
// output is the same on every machine. Reference `node` is NOT invariant, which
// is why run.sh pins the environment before comparing — without that pin this
// file would pass on one developer's box and fail on another's.
console.log((1234567.891).toLocaleString());
console.log((0).toLocaleString(), (-0).toLocaleString(), (123).toLocaleString());
console.log((1e21).toLocaleString(), (1e30).toLocaleString());
console.log(NaN.toLocaleString(), Infinity.toLocaleString(), (-Infinity).toLocaleString());
console.log((1234567n).toLocaleString(), (-1234567n).toLocaleString(), (0n).toLocaleString());
console.log([1234.5, 'x', null, undefined].toLocaleString());
console.log(({}).toLocaleString(), ({ toString() { return 'T'; } }).toLocaleString());
console.log('abc'.toLocaleString(), true.toLocaleString(), true.toString());
console.log('straße'.toLocaleUpperCase(), 'ÄÖÜ'.toLocaleLowerCase());
console.log(new Date(0).toLocaleString());
console.log(new Date(0).toLocaleDateString(), new Date(0).toLocaleTimeString());
console.log(new Date(1700000000123).toLocaleString());
console.log(new Date(Date.UTC(2020, 0, 2, 15, 4, 5)).toLocaleTimeString());
console.log(new Date(NaN).toLocaleString());
console.log('a'.normalize(), 'a'.normalize('NFKD'));
try {
  'a'.normalize('XX');
} catch (e) {
  console.log(e.constructor.name + ': ' + e.message);
}
