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

// An accessor redefined as a data property. The value used to be written while
// the accessor stayed in its side table, and accessors win on read — so the
// getter kept answering and the descriptor still reported get/set.
const conv = { a: 1, get b() { return 'getter'; }, c: 3 };
console.log('pre     ', Object.keys(conv).join(','), conv.b);
Object.defineProperty(conv, 'b', { value: 9 });
console.log('post    ', Object.keys(conv).join(','), conv.b);
console.log('kind    ', Object.keys(Object.getOwnPropertyDescriptor(conv, 'b')).sort().join(','));
// The key keeps its place in the own-key order across that conversion: the
// accessor's position is recorded by a marker, not by a real key, so a naive
// delete-and-insert would move `b` to the end.

// A data property converted the other way.
const back = {};
Object.defineProperty(back, 'd', { value: 1, configurable: true });
Object.defineProperty(back, 'd', { get() { return 'now-accessor'; } });
console.log('data->acc', back.d);

// A flags-only redefinition leaves an accessor an accessor.
const flags = { get e() { return 'still'; } };
Object.defineProperty(flags, 'e', { enumerable: false });
console.log('flagsonly', flags.e, Object.keys(flags).length);

// An accessor installed through defineProperty carries no `writable` field, so
// its stored attribute is false — and the write path used to test that before
// looking for a setter, silently swallowing every assignment. 10.1.9.2 branches
// on the descriptor kind first: on an accessor the setter alone decides. This
// broke the standard clone idiom below, whose setters did nothing.
const orig = { v: 1, get p() { return 'P'; }, set p(x) { this.seen = x; } };
const copy = Object.create(Object.getPrototypeOf(orig), Object.getOwnPropertyDescriptors(orig));
copy.p = 'written';
console.log('clone   ', copy.v, copy.p, copy.seen);
const direct = {};
Object.defineProperty(direct, 'q', { get() { return 'Q'; }, set(x) { this.got = x; } });
direct.q = 'sent';
console.log('direct  ', direct.q, direct.got);
// A getter with no setter still swallows the write rather than throwing.
const readonly = {};
Object.defineProperty(readonly, 'r', { get() { return 'R'; } });
readonly.r = 'ignored';
console.log('getonly ', readonly.r);

// Annex B B.2.2.2-B.2.2.5. Legacy, but node has them and pre-defineProperty
// libraries still reach for them; all four were missing entirely.
const legacy = {};
legacy.__defineGetter__('g', function () { return 'got'; });
legacy.__defineSetter__('g', function (x) { this.stored = x; });
legacy.g = 'set-through-legacy';
console.log('legacy  ', legacy.g, legacy.stored, Object.keys(legacy).join(','));
console.log('lookup  ', typeof legacy.__lookupGetter__('g'), typeof legacy.__lookupSetter__('g'));
// The lookups walk the prototype chain, unlike getOwnPropertyDescriptor.
const heir = Object.create(legacy);
console.log('inherit ', typeof heir.__lookupGetter__('g'), Object.prototype.hasOwnProperty.call(heir, 'g'));
console.log('missing ', legacy.__lookupGetter__('nope'), typeof {}.__defineGetter__);

// `delete obj.accessorProp` reported success and removed nothing. An accessor
// lives in its own table rather than the property map, and the delete cleared
// only the map — so the getter kept answering and `in` kept reporting the key.
const removable = { a: 1, get b() { return 2; }, c: 3 };
console.log("del-pre ", Object.keys(removable).join(","), removable.b);
console.log("del-ok  ", delete removable.b, "b" in removable, String(removable.b));
console.log("del-post", Object.keys(removable).join(","), JSON.stringify(removable));
// A getter that deletes itself mid-read: the second read finds nothing.
const once = { get value() { delete once.value; return "first"; } };
console.log("selfdel ", once.value, String(once.value), "value" in once);
// Both spellings and Reflect take the same path.
const viaComputed = { get g() { return 1; } };
const viaReflect = { get g() { return 1; } };
console.log("spellings", delete viaComputed["g"], Reflect.deleteProperty(viaReflect, "g"), "g" in viaComputed, "g" in viaReflect);
// A non-configurable accessor resists, as a non-configurable data property does.
const pinned = {};
Object.defineProperty(pinned, "k", { get: () => 1, configurable: false });
console.log("nonconf ", delete pinned.k, "k" in pinned, pinned.k);
// After removal the name is free for an ordinary assignment.
const reused = { get x() { return "accessor"; } };
delete reused.x;
reused.x = "plain";
console.log("readd   ", reused.x, JSON.stringify(reused));
// Deleting an own accessor uncovers one inherited from the prototype.
const base = { get p() { return "base"; } };
const derived = Object.create(base);
Object.defineProperty(derived, "p", { get: () => "own", configurable: true });
delete derived.p;
console.log("shadow  ", derived.p, Object.prototype.hasOwnProperty.call(derived, "p"));
