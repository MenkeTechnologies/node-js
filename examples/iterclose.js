// Consumers that DRAINED an iterator before validating what it yielded. Over an
// infinite source that meant the check was never reached and the call HUNG —
// `Array.from(infinite, throwingMapper)` and `new Map(infinite)` both ran
// forever. The spec steps the iterator and acts per element.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
// An iterator that never ends, and logs what is asked of it.
const endless = () => {
  const log = [];
  return {
    log,
    it: { i: 0, [Symbol.iterator]() { return this; },
      next() { log.push("next"); return { value: this.i++, done: false }; },
      return() { log.push("return"); return { done: true }; } },
  };
};
show("from-throws  ", () => { const { it, log } = endless(); try { Array.from(it, () => { throw new Error("m"); }); } catch (e) { return [e.message, log]; } });
show("from-maps    ", () => Array.from([1, 2], (v, i) => v * 10 + i));
show("from-order   ", () => { const seen = []; Array.from({ [Symbol.iterator]() { let i = 0; return { next: () => (i < 2 ? { value: i++, done: false } : { done: true }) }; } }, (v) => { seen.push("map" + v); return v; }); return seen; });
show("map-bad-entry", () => { const { it, log } = endless(); try { new Map(it); } catch (e) { return [e.message, log]; } });
show("weakmap-bad  ", () => { try { new WeakMap([1]); } catch (e) { return e.message; } });
// A STRING is iterable, so an entry check that only tried to iterate accepted
// one and stored `'a' => 'b'`.
show("map-string   ", () => { try { new Map(["ab"]); } catch (e) { return e.message; } });
show("map-pairs    ", () => [...new Map([["k", "v"]])]);
show("map-arraylike", () => [...new Map([{ 0: "k", 1: "v", length: 2 }])]);
show("map-short    ", () => [...new Map([[]])]);
show("set-values   ", () => [...new Set([1, 2, 2])]);

// A combinator REJECTS when the iterable misbehaves; it does not throw at the
// call site, so a `.catch()` can still attach.
const badIterable = { [Symbol.iterator]() { return this; }, next() { throw new TypeError("boom"); } };
for (const [name, make] of [["all", () => Promise.all(badIterable)],
  ["allSettled", () => Promise.allSettled(badIterable)],
  ["race", () => Promise.race(badIterable)],
  ["any", () => Promise.any(badIterable)],
  ["non-iterable", () => Promise.all({})]]) {
  try { make().catch((e) => console.log(("reject-" + name).padEnd(18), e.constructor.name)); }
  catch (e) { console.log(("reject-" + name).padEnd(18), "THREW", e.constructor.name); }
}
Promise.all([Promise.resolve(1), 2]).then((v) => console.log("all-ok        ", JSON.stringify(v)));

// IteratorClose on the paths that already had it, pinned so they stay.
const closing = () => { const log = []; return { log, it: { [Symbol.iterator]() { return this; }, next() { log.push("next"); return { value: 1, done: false }; }, return() { log.push("return"); return { done: true }; } } }; };
show("break        ", () => { const { it, log } = closing(); for (const v of it) break; return log; });
show("return       ", () => { const { it, log } = closing(); (function () { for (const v of it) return; })(); return log; });
show("destructure  ", () => { const { it, log } = closing(); const [a] = it; return [a, log]; });
show("spread-finite", () => { const log = []; const it = { i: 0, [Symbol.iterator]() { return this; }, next() { log.push("next"); return this.i < 2 ? { value: this.i++, done: false } : { done: true }; }, return() { log.push("return"); return { done: true }; } }; return [[...it], log]; });
show("generator    ", () => { const log = []; function* g() { try { yield 1; yield 2; } finally { log.push("finally"); } } for (const v of g()) break; return log; });
show("no-return    ", () => { const it = { [Symbol.iterator]() { return this; }, next() { return { value: 1, done: false }; } }; for (const v of it) break; return "ok"; });
show("return-throws", () => { const it = { [Symbol.iterator]() { return this; }, next() { return { value: 1, done: false }; }, return() { throw new TypeError("ret"); } }; try { for (const v of it) break; } catch (e) { return e.constructor.name; } });
