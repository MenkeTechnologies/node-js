// `sort` is the last array method that wrote back a whole replacement copy.
// 23.1.3.30 steps 4-5 write only the indices BELOW the length captured at step
// 1: a `Set` for each sorted element, then a `Delete` for the holes that
// followed them. Anything the COMPARATOR appended past that length survives —
// replacing the backing store discarded it.
//
// Nothing here counts comparator CALLS: the number of comparisons is not
// specified, and node itself uses different counts for `sort` and `toSorted` on
// the same input. Every mutation below is guarded to happen exactly once.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

show("append-once  ", () => { const a = [3, 1, 2]; let once = true; a.sort((x, y) => { if (once) { once = false; a.push(7, 8); } return x - y; }); return [a.length, a]; });
show("shrink-once  ", () => { const a = [3, 1, 2, 5, 4]; let once = true; a.sort((x, y) => { if (once) { once = false; a.length = 2; } return x - y; }); return [a.length, a]; });
// A hole sorts to the END, after the `undefined`s, and stays a hole — the
// element count and the length are two different numbers here.
show("holes        ", () => { const a = [3, , 1, undefined, 2]; a.sort(); return [a.length, a, 3 in a, 4 in a]; });
show("holes-cmp    ", () => { const a = [3, , 1, undefined, 2]; a.sort((x, y) => x - y); return [a.length, 3 in a, 4 in a]; });
// `undefined` is never handed to the comparator (23.1.3.30.1 step 3.c) — it
// sorts to the end after the defined values are ordered. Which PAIR the
// comparator sees, and in which argument order, is up to the sort algorithm, so
// only the absence of `undefined` is asserted.
show("undef-nocall ", () => { const seen = []; const a = [3, undefined, 1, undefined, 2].sort((x, y) => { seen.push(x, y); return x - y; }); return [seen.some((v) => v === undefined), a]; });
// The sort is stable, and a comparator returning NaN leaves the order alone.
show("stable       ", () => { const a = [{ k: 1, i: 0 }, { k: 0, i: 1 }, { k: 1, i: 2 }, { k: 0, i: 3 }]; a.sort((x, y) => x.k - y.k); return a.map((o) => o.i); });
show("nan-compare  ", () => [3, 1, 2].sort(() => NaN));
// The default comparator is ToString order, and `undefined` still goes last.
show("default      ", () => [10, 9, 1, undefined, null, 2].sort());
// A throwing comparator propagates, and 23.1.3.30 step 1 rejects a
// non-callable BEFORE any comparison. V8 renders the offending value with
// `NoSideEffectsToString`, so a string is bare and an array is `[object Array]`
// — `util.inspect` would have quoted the one and expanded the other.
show("throws       ", () => { try { [3, 1, 2].sort(() => { throw new Error("boom"); }); } catch (e) { return e.message; } });
show("reject-string", () => { try { return [1, 2].sort("x"); } catch (e) { return e.message; } });
show("reject-null  ", () => { try { return [1, 2].sort(null); } catch (e) { return e.message; } });
show("reject-array ", () => { try { return [1, 2].sort([1, 2]); } catch (e) { return e.message; } });
show("reject-object", () => { try { return [1, 2].sort({}); } catch (e) { return e.message; } });
show("reject-regexp", () => { try { return [1, 2].sort(/re/); } catch (e) { return e.message; } });
show("reject-symbol", () => { try { return [1, 2].sort(Symbol("s")); } catch (e) { return e.message; } });
// `toSorted` leaves the receiver alone, so an appending comparator grows the
// source and not the result.
show("toSorted     ", () => { const a = [3, 1, 2]; let once = true; const b = a.toSorted((x, y) => { if (once) { once = false; a.push(9); } return x - y; }); return [b, a.length, a[3]]; });
