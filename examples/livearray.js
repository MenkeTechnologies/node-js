// Two things a callback can do that the implementation was blind to: MUTATE
// the array it is iterating, and build a prototype CYCLE.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// An `Array.prototype` iteration method captures the LENGTH once (step 3) but
// reads each element LIVE at its index, skipping one a shrinking mutation has
// dropped. Snapshotting the whole array meant none of this was observed.
show("shift-forEach", () => { const a = [1, 2, 3]; const seen = []; a.forEach((v) => { seen.push(v); a.shift(); }); return [seen, a]; });
show("pop-forEach  ", () => { const a = [1, 2, 3]; const seen = []; a.forEach((v) => { seen.push(v); a.pop(); }); return [seen, a]; });
show("push-forEach ", () => { const a = [1]; const seen = []; let n = 0; a.forEach((v) => { seen.push(v); if (++n < 4) a.push(9); }); return [seen, a.length]; });
show("splice-map   ", () => { const a = [1, 2, 3]; const r = a.map((v, i) => { if (i === 0) a.splice(1, 1); return v; }); return [r, a]; });
show("filter-shrink", () => { const a = [1, 2, 3]; return [a.filter((v, i) => { if (i === 0) a.length = 1; return true; }), a]; });
show("reduce-shrink", () => { const a = [1, 2, 3]; return a.reduce((acc, v, i) => { if (i === 0) a.length = 1; return acc + v; }, 0); });
show("every-shrink ", () => { const a = [1, 2, 3]; let calls = 0; const r = a.every((v, i) => { calls++; if (i === 0) a.length = 1; return true; }); return [r, calls]; });
// A HOLE is skipped without calling the callback, and `map` keeps it a hole.
show("holes        ", () => { const a = [1, , 3]; const seen = []; a.forEach((v, i) => seen.push(i)); return [seen, a.map((v) => v * 2), 1 in a.map((v) => v)]; });
show("map-length   ", () => { const a = [1, 2, 3]; const r = a.map((v, i) => { if (i === 0) a.length = 1; return v; }); return [r, r.length]; });
// And the ordinary cases, unchanged.
show("ordinary     ", () => [[1, 2, 3].map((v) => v * 2), [1, 2, 3].filter((v) => v > 1),
  [1, 2, 3].reduce((a, b) => a + b), [1, 2, 3].every((v) => v > 0), [1, 2, 3].some((v) => v > 2)]);
show("index-arg    ", () => { const seen = []; ["a", "b"].forEach((v, i, arr) => seen.push([v, i, arr.length])); return seen; });
show("thisArg      ", () => { const out = []; [1].forEach(function () { out.push(this.tag); }, { tag: "T" }); return out; });

// 10.1.2.1 step 8: a prototype assignment that would create a CYCLE is
// refused. Nothing hung without the check — every chain walk here carries a hop
// limit — but a lookup then silently gave up instead of finding a property
// that really was there.
show("cycle-two    ", () => { const a = {}, b = {}; Object.setPrototypeOf(a, b); Object.setPrototypeOf(b, a); return "set"; });
show("cycle-self   ", () => { const a = {}; Object.setPrototypeOf(a, a); return "set"; });
show("cycle-three  ", () => { const a = {}, b = {}, c = {}; Object.setPrototypeOf(a, b); Object.setPrototypeOf(b, c); Object.setPrototypeOf(c, a); return "set"; });
show("cycle-setter ", () => { const a = {}, b = {}; a.__proto__ = b; b.__proto__ = a; return "set"; });
// `Reflect` reports the refusal rather than throwing, as it does for a
// non-extensible receiver.
show("cycle-reflect", () => { const a = {}, b = {}; Reflect.setPrototypeOf(a, b); return Reflect.setPrototypeOf(b, a); });
// What must still be allowed.
show("chain-ok     ", () => { const a = {}, b = { v: 1 }; Object.setPrototypeOf(a, b); return [Object.getPrototypeOf(a) === b, a.v]; });
show("null-ok      ", () => { const a = {}; Object.setPrototypeOf(a, null); return Object.getPrototypeOf(a); });
show("relink-ok    ", () => { const a = {}, b = {}, c = { v: 2 }; Object.setPrototypeOf(a, b); Object.setPrototypeOf(a, c); return a.v; });
show("deep-ok      ", () => { let p = null; for (let i = 0; i < 20; i++) { const o = {}; Object.setPrototypeOf(o, p); p = o; } p.tip = 1; return p.tip; });
