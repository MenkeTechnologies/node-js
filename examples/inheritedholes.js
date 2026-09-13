// A HOLE is not an absent value, it is an absent OWN property: `[[Get]]` keeps
// walking the prototype chain, and every array method tests `HasProperty`
// before deciding to skip a position. The elision short-circuited both, so an
// array inherited nothing at an elided index — and, for a DATA property,
// nothing at any index at all. An accessor on the prototype was already found,
// which is what hid it.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const keys = (o) => { const a = []; for (const k in o) a.push(k); return a; };

// An explicit prototype: a hole, an index past the end and a named key all
// resolve through it. `in` already answered true for each, so the read and the
// operator were disagreeing about the same question.
const proto = { 1: "q", 5: "v", z: "s" };
const linked = () => { const a = [1, , 3]; Object.setPrototypeOf(a, proto); return a; };
show("hole-read    ", () => { const a = linked(); return [a[1], 1 in a, a.hasOwnProperty(1)]; });
show("past-end     ", () => { const a = linked(); return [a[5], 5 in a]; });
show("named        ", () => { const a = linked(); return [a.z, "z" in a]; });
show("own-wins     ", () => { const a = linked(); return [a[0], a[2]]; });
show("accessor     ", () => { const a = [1, , 3]; Object.setPrototypeOf(a, { get 1() { return "acc"; } }); return a[1]; });

// The same through the intrinsic prototype, which is where real code patches.
const patched = (f) => { Array.prototype[1] = "p"; try { return f(); } finally { delete Array.prototype[1]; } };
show("intrinsic    ", () => patched(() => { const a = [1, , 3]; return [a[1], 1 in a, a.hasOwnProperty(1)]; }));
show("intrinsic-end", () => { Array.prototype[9] = "p"; const a = [1, 2]; const r = [a[9], 9 in a]; delete Array.prototype[9]; return r; });
// It is still not an OWN key, so the own-key listings do not change…
show("not-own      ", () => patched(() => { const a = [1, , 3]; return [Object.keys(a), Object.getOwnPropertyNames(a), a.length]; }));
// …but `for-in` visits it, after the own keys, because it walks the chain.
show("for-in       ", () => patched(() => keys([1, , 3])));
show("for-in-dense ", () => patched(() => keys([1, 2])));
// Every consumer that tests HasProperty now sees the position as present, and
// reads the INHERITED value rather than the hole's `undefined`.
show("map          ", () => patched(() => [1, , 3].map((v) => v)));
show("forEach      ", () => patched(() => { const seen = []; [1, , 3].forEach((v, i) => seen.push([i, v])); return seen; }));
show("filter       ", () => patched(() => [1, , 3].filter(() => true)));
show("flat         ", () => patched(() => [1, , 3].flat()));
show("spread       ", () => patched(() => [...[1, , 3]]));
show("json         ", () => patched(() => JSON.stringify([1, , 3])));
show("join         ", () => patched(() => [1, , 3].join(",")));
show("indexOf      ", () => patched(() => [[1, , 3].indexOf("p"), [1, , 3].includes("p"), [1, , 3].lastIndexOf("p")]));
show("sort         ", () => patched(() => [3, , 1].sort()));
show("reduce       ", () => patched(() => [1, , 3].reduce((a, b) => a + "" + b)));
show("concat-slice ", () => patched(() => [[0].concat([1, , 3]), [1, , 3].slice(1)]));
// A hole with NOTHING on the chain is still absent everywhere.
show("plain-hole   ", () => { const a = [1, , 3]; return [a[1], 1 in a, a.map((v) => v), JSON.stringify(a), a.flat(), keys(a)]; });
