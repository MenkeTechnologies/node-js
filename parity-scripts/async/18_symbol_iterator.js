// Custom iterable via Symbol.iterator; spread and for-of consume it.
const range = {
  from: 1,
  to: 5,
  [Symbol.iterator]() {
    let cur = this.from;
    const last = this.to;
    return {
      next() {
        return cur <= last
          ? { value: cur++, done: false }
          : { value: undefined, done: true };
      },
    };
  },
};

const collected = [];
for (const n of range) collected.push(n);
console.log('for-of=' + collected.join(','));
console.log('spread=' + [...range].join(','));
console.log('sum=' + [...range].reduce((a, b) => a + b, 0));

// Symbol identity + description.
const s1 = Symbol('tag');
const s2 = Symbol('tag');
console.log('unique=' + (s1 !== s2));
console.log('desc=' + s1.description);
console.log('for-registry=' + (Symbol.for('k') === Symbol.for('k')));
console.log('keyFor=' + Symbol.keyFor(Symbol.for('k')));
