// The array methods `livearray.js` did not reach. Each one captures the LENGTH
// once and then reads every element LIVE, so a callback, getter or `toString`
// that mutates the array is observed by every step that follows it. All seven
// read a snapshot taken before the first call, which made the mutation
// invisible.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// `some` stops at the first truthy result, and an index a shrink has dropped is
// skipped rather than served stale: three calls became one.
show("some-shrink  ", () => { const a = [1, 2, 3]; let calls = 0; const r = a.some((v, i) => { calls++; if (i === 0) a.length = 1; return false; }); return [r, calls]; });
show("some-hit     ", () => { const a = [1, 2, 3]; const seen = []; const r = a.some((v) => { seen.push(v); return v === 2; }); return [r, seen]; });
// `flatMap` flattens one level, and drops the steps a shrink removed.
show("flatMap-shrnk", () => { const a = [1, 2, 3]; let calls = 0; const r = a.flatMap((v, i) => { calls++; if (i === 0) a.length = 1; return [v]; }); return [r, calls]; });
show("flatMap-depth", () => [1, 2].flatMap((v) => [[v]]));
// `reduceRight` walks DOWN, so a shrink during the first (highest-index) call
// removes the middle of the walk.
show("reduceR-shrnk", () => { const a = [1, 2, 3]; let calls = 0; const r = a.reduceRight((acc, v) => { calls++; if (calls === 1) a.length = 1; return acc + v; }, 0); return [r, calls]; });
show("reduceR-noini", () => [1, 2, 3].reduceRight((acc, v) => acc + "" + v));
show("reduceR-empty", () => { try { return [].reduceRight((a, b) => a + b); } catch (e) { return e.message; } });
// `indexOf` does HasProperty before Get (23.1.3.17 step 8a), so an index the
// getter's mutation dropped is not compared at all — the answer is -1, not the
// position the element used to have.
show("indexOf-getter", () => { const a = [1, 2, 3]; Object.defineProperty(a, "0", { configurable: true, get() { a.length = 1; return 1; } }); return a.indexOf(3); });
show("indexOf-hole  ", () => [[1, , 3].indexOf(undefined), [1, , 3].lastIndexOf(undefined)]);
// `includes` has NO HasProperty step (23.1.3.16 step 5b): it reads through and
// compares `undefined`, which is why a hole matches where `indexOf` misses.
show("includes-gettr", () => { const a = [1, 2, 3]; Object.defineProperty(a, "0", { configurable: true, get() { a.length = 1; return 1; } }); return a.includes(3); });
show("includes-hole ", () => [[1, , 3].includes(undefined), [NaN].includes(NaN), [NaN].indexOf(NaN)]);
// `join` reads AND stringifies one element before reading the next, so both a
// mutating getter and a mutating `toString` are seen by the rest of the walk.
// Past the shrunken length every read is `undefined`, which joins as empty.
show("join-getter   ", () => { const a = [1, 2, 3]; Object.defineProperty(a, "0", { configurable: true, get() { a.length = 1; return "x"; } }); return a.join(","); });
show("join-toString ", () => { const a = [{ toString() { a.length = 1; return "x"; } }, 2, 3]; return a.join(","); });
show("join-basics   ", () => [[1, , 3].join("-"), [null, undefined, 0].join(","), [[1, 2], 3].join(";")]);
