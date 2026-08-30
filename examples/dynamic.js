// Dynamic dispatch: the receiver a call binds, resolved through a computed key
// rather than a literal one. `recv[expr](…)` and `recv.name(…)` are the same
// operation in 13.3.6 EvaluateCall and must agree on `this`.

const obj = {
  n: 42,
  self() { return this.n; },
  add(a, b) { return this.n + a + b; },
};

// Literal, string-computed, and variable-computed all name the same method.
const key = 'self';
console.log('direct  ', obj.self());
console.log('literal ', obj['self']());
console.log('variable', obj[key]());
console.log('computed', obj[['se', 'lf'].join('')]());
console.log('args    ', obj['add'](1, 2), obj.add(1, 2));

// Class instances, including an inherited method reached the same way.
class Base { constructor(v) { this.v = v; } value() { return this.v; } }
class Derived extends Base { doubled() { return this.value() * 2; } }
const d = new Derived(7);
console.log('class   ', d.value(), d['value'](), d['doubled']());

// Builtins: the receiver has to survive for these to mean anything.
console.log('array   ', JSON.stringify([3, 1, 2]['sort']()), [1, 2]['concat']([3]).join(''));
console.log('string  ', 'abc'['toUpperCase'](), 'a,b'['split'](','). length);
console.log('buffer  ', Buffer.from('hi')['toString'](), Buffer.from('hi')['slice'](0, 1).toString());

// An element that happens to be a function: the key is an INDEX, and `this` is
// the array, so `length` is visible.
const fns = [function () { return this.length; }, function () { return 'second'; }];
console.log('index   ', fns[0](), fns[1](), fns['1']());

// Spread through the same path.
console.log('spread  ', obj['add'](...[3, 4]), Math['max'](...[1, 9, 5]));

// call/apply/bind still bind explicitly, and must not be disturbed.
console.log('explicit', obj.self.call({ n: 1 }), obj.self.apply({ n: 2 }), obj.self.bind({ n: 3 })());

// Optional computed calls.
const maybe = null;
console.log('optional', maybe?.['self']?.(), obj?.['self']?.());

// A method looked up once and invoked later keeps no receiver, as in JS.
const loose = obj['self'];
console.log('detached', typeof loose, loose.call(obj));
