const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const lk of [{ length: 1.9, 0: 4 }, { length: 3, 0: 1, 2: 3 }, { length: '2', 0: 'a', 1: 'b' }, { length: -1 }, { length: NaN, 0: 1 }, { length: 3, 1: 'x' }]) {
  for (const m of ['push', 'pop', 'shift', 'unshift', 'reverse', 'sort', 'splice', 'fill', 'copyWithin']) {
    const o = Object.assign({}, lk);
    const args = { push: [9], unshift: [0], splice: [0, 1], fill: ['f'], copyWithin: [0, 1] }[m] || [];
    console.log(JSON.stringify(lk), m, P(() => Array.prototype[m].apply(o, args)), JSON.stringify(o), Object.keys(o).join());
  }
}
console.log(P(() => Array.prototype.push.call('abc', 1)));
class MyArr extends Array {}
const mkB = (S) => { class B extends Array { static get [Symbol.species]() { return S; } } return B.from([1, 2, 3]); };
const desc = (d) => Object.prototype.toString.call(d) + ':' + (d instanceof Array) + ':' + d.length + ':' + Object.keys(d).join();
for (const [label, S] of [['Object', Object], ['one', 1], ['fnLen', function (n) { return { length: n }; }], ['fn0', function () { return { length: 0 }; }], ['Array', Array], ['undef', undefined], ['null', null], ['arrow', () => {}]]) {
  for (const m of ['map((x) => x)', 'filter(() => true)', 'slice()', 'splice(0, 1)', 'concat([1])', 'flat()', 'flatMap((x) => [x])']) {
    console.log(label, m, P(() => desc(eval('mkB(S).' + m))));
  }
}
