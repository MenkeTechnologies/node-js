// Accessors, descriptors and the prototype chain — none of which the corpus
// covered. `dynamic.js` pinned how a call finds its receiver; this pins how a
// READ finds its value: getters and setters, `defineProperty`, enumerability,
// descriptors, class and static accessors, and prototype shadowing.

const o = {
  _v: 1,
  get v() { return this._v; },
  set v(n) { this._v = n * 2; },
};
o.v = 5;
console.log('literal ', o.v, o._v);

// Reached through a computed key, which must still bind `this`.
const k = 'v';
o[k] = 10;
console.log('computed', o[k], o['_v']);

// defineProperty: accessor, and a non-enumerable data property.
const d = {};
Object.defineProperty(d, 'twice', { get() { return this.n * 2; }, enumerable: true });
Object.defineProperty(d, 'n', { value: 4, writable: true, enumerable: false });
console.log('defined ', d.twice, Object.keys(d).join(','), JSON.stringify(d));

// Descriptors report what was set.
const dd = Object.getOwnPropertyDescriptor(d, 'n');
console.log('descr   ', dd.value, dd.writable, dd.enumerable, dd.configurable);
console.log('accessor', typeof Object.getOwnPropertyDescriptor(d, 'twice').get);

// Class accessors, including static and inherited.
class Base {
  constructor() { this._x = 3; }
  get x() { return this._x; }
  set x(n) { this._x = n + 1; }
  static get kind() { return 'base'; }
}
class Sub extends Base {
  get doubled() { return this.x * 2; }
}
const s = new Sub();
s.x = 9;
// `Sub.kind` reads through `extends`: a static ACCESSOR is inherited the same
// way a static method is. It was not, until the read learned to walk the class
// parent chain for accessors as it already did for methods and fields.
console.log('class   ', s.x, s.doubled, Base.kind, Sub.kind);

// The accessor lives on the prototype, not the instance.
console.log('where   ', Object.getOwnPropertyNames(s).join(','),
            typeof Object.getOwnPropertyDescriptor(Base.prototype, 'x').get);

// Prototype chain lookups and shadowing.
const proto = { greet() { return 'proto'; } };
const child = Object.create(proto);
console.log('chain   ', child.greet(), Object.getPrototypeOf(child) === proto);
child.greet = () => 'own';
console.log('shadow  ', child.greet(), proto.greet());
