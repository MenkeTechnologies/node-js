// The built-in iterators inherit from their kind's intrinsic prototype
// (%ArrayIteratorPrototype%, %MapIteratorPrototype%, %SetIteratorPrototype%,
// %StringIteratorPrototype%, %RegExpStringIteratorPrototype%), each of which
// owns `next` and a tag and inherits from Iterator.prototype. Every built-in
// iterator reported Iterator.prototype itself as its prototype, so the
// standard probe for %IteratorPrototype% answered false.

const IP = Iterator.prototype;
const its = {
  array: [1][Symbol.iterator](),
  arrayKeys: [1].keys(),
  typed: new Uint8Array(1).values(),
  buffer: Buffer.alloc(1).entries(),
  args: (function () { return arguments[Symbol.iterator](); })(1),
  map: new Map([[1, 2]]).entries(),
  mapKeys: new Map().keys(),
  set: new Set([1]).values(),
  setEntries: new Set().entries(),
  string: "ab"[Symbol.iterator](),
  regexp: "a".matchAll(/a/g),
};
for (const [k, it] of Object.entries(its)) {
  const p = Object.getPrototypeOf(it);
  console.log(
    k.padEnd(10),
    Object.getPrototypeOf(p) === IP,
    it instanceof Iterator,
    it[Symbol.toStringTag],
    Reflect.ownKeys(p).map(String).join(),
    p.next.name,
    p.next.length,
    require("util").inspect(p),
  );
}
const AIP = Object.getPrototypeOf([].values());
console.log(Object.getPrototypeOf(new Uint8Array(1).values()) === AIP);
console.log(Object.getPrototypeOf(new Set().keys()) === Object.getPrototypeOf(new Set().entries()));
console.log(Object.getOwnPropertyDescriptor(AIP, Symbol.toStringTag));
console.log(Object.getOwnPropertyDescriptor(AIP, "next").writable, AIP[Symbol.iterator]() === AIP);

// `next` through the prototype: brand-checked per kind.
console.log(AIP.next.call([5].values()), Object.getPrototypeOf(new Map([[1, 2]]).keys()).next.call(new Map([[1, 2]]).keys()));
const calls = [
  () => AIP.next.call(new Map().keys()),
  () => AIP.next.call({}),
  () => Object.getPrototypeOf(new Map().keys()).next.call([].values()),
  () => Object.getPrototypeOf(new Set().keys()).next.call({}),
  () => Object.getPrototypeOf(""[Symbol.iterator]()).next.call({}),
  () => Object.getPrototypeOf("a".matchAll(/a/g)).next.call({}),
];
for (const f of calls) {
  try {
    f();
  } catch (e) {
    console.log(e.constructor.name, e.message);
  }
}

// The chain is live: a patched prototype is seen by every iterator of the kind.
AIP.extra = 7;
console.log([].values().extra, "extra" in [].values());
delete AIP.extra;
const MIP = Object.getPrototypeOf(new Map().keys());
MIP.extra = 8;
console.log(new Map().keys().extra);
delete MIP.extra;
const saved = AIP.next;
AIP.next = function () { return { value: "patched", done: false }; };
console.log([1].values().next());
AIP.next = saved;
console.log([1].values().next());

// Iterator.prototype's own members reach an ordinary object that inherits it.
const o = Object.create(Iterator.prototype);
let i = 0;
o.next = () => ({ value: i++, done: i > 3 });
console.log(o[Symbol.iterator]() === o, o.map((x) => x * 10).toArray(), [...o]);
const GP = Object.getPrototypeOf(function* () {}).prototype;
console.log(GP[Symbol.iterator]() === GP);

// A member patched onto an intrinsic prototype handle is inherited through an
// explicit chain too.
Iterator.prototype.q = 2;
console.log(Object.create(Iterator.prototype).q, GP.q, (function* () {})().q, new Map().keys().q, [].values().q, "q" in new Set().values());
delete Iterator.prototype.q;
Array.prototype.zz = 1;
const oa = Object.create(Array.prototype);
console.log(oa.zz, "zz" in oa, Object.create(oa).zz);
delete Array.prototype.zz;
console.log(oa.zz, "zz" in oa);
for (const x of [1, 2]) process.stdout.write(x + " ");
for (const [k] of new Map([[3, 4]])) process.stdout.write(k + " ");
for (const ch of "yz") process.stdout.write(ch);
console.log();
