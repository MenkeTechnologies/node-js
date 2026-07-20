// Custom iterators via Symbol.iterator.
class LinkedList {
  constructor() {
    this.head = null;
  }
  prepend(value) {
    this.head = { value, next: this.head };
    return this;
  }
  [Symbol.iterator]() {
    let node = this.head;
    return {
      next() {
        if (node) {
          const value = node.value;
          node = node.next;
          return { value, done: false };
        }
        return { value: undefined, done: true };
      },
    };
  }
}

const list = new LinkedList();
list.prepend(3).prepend(2).prepend(1);
console.log("iterate:", [...list].join(","));
console.log("spread into args:", Math.max(...list));

for (const v of list) {
  process.stdout.write(`[${v}]`);
}
process.stdout.write("\n");

// Range object as iterable.
const range = {
  from: 1,
  to: 5,
  [Symbol.iterator]() {
    let current = this.from;
    const last = this.to;
    return {
      next() {
        return current <= last
          ? { value: current++, done: false }
          : { value: undefined, done: true };
      },
    };
  },
};
console.log("range sum:", [...range].reduce((a, b) => a + b, 0));
