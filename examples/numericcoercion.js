// `Array.prototype`, `Number.prototype`, `Math` and `parseInt` coerce their
// numeric arguments with `ToNumber`, which runs a user `valueOf` and can throw
// from it. None of them ran one: the read they used does no `ToPrimitive` at
// all, so `[1,2,3].slice({valueOf: () => 1})` sliced from 0 and
// `(1.234).toFixed(obj)` was a RangeError.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
// Each probe reports which conversion RAN, so an argument that lands on the
// right answer without being coerced is still visible.
const trace = (call) => {
  const log = [];
  const mk = (tag) => ({ valueOf() { log.push(tag + ":valueOf"); return 1; }, toString() { log.push(tag + ":toString"); return "1"; } });
  let out;
  try { out = call(mk); } catch (e) { out = e.constructor.name; }
  return [log, out];
};

// Array index positions.
show("slice-start  ", () => trace((m) => [1, 2, 3].slice(m("a"))));
show("slice-end    ", () => trace((m) => [1, 2, 3].slice(0, m("a"))));
show("splice-start ", () => trace((m) => [1, 2, 3].splice(m("a"))));
show("indexOf-from ", () => trace((m) => [1, 2, 3].indexOf(3, m("a"))));
show("includes-from", () => trace((m) => [1, 2, 3].includes(3, m("a"))));
show("fill-start   ", () => trace((m) => [1, 2, 3].fill(0, m("a"))));
show("copyWithin   ", () => trace((m) => [1, 2, 3].copyWithin(m("a"))));
show("at           ", () => trace((m) => [1, 2, 3].at(m("a"))));
show("flat-depth   ", () => trace((m) => [[1], [2]].flat(m("a"))));
show("with-index   ", () => trace((m) => [1, 2, 3].with(m("a"), 9)));
show("toSpliced    ", () => trace((m) => [1, 2, 3].toSpliced(m("a"), 1)));
// …and the VALUE positions of the same methods, which must NOT be coerced.
show("fill-value   ", () => trace((m) => [1, 2].fill(m("a"))));
show("with-value   ", () => trace((m) => [1, 2].with(0, m("a"))));
show("splice-items ", () => trace((m) => [1, 2].splice(0, 1, m("a"))));
show("indexOf-value", () => trace((m) => [1, 2].indexOf(m("a"))));
show("push-value   ", () => trace((m) => { const a = []; a.push(m("a")); return a.length; }));
// `Number.prototype`, where the argument is a digit count or a radix.
show("toFixed      ", () => trace((m) => (1.234).toFixed(m("a"))));
show("toExponential", () => trace((m) => (1.234).toExponential(m("a"))));
show("toPrecision  ", () => trace((m) => (1.234).toPrecision(m("a"))));
show("toString-radx", () => trace((m) => (255).toString(m("a"))));
// `Math` coerces EVERY argument, and the variadic ones must not lose any.
show("math-abs     ", () => trace((m) => Math.abs(m("a"))));
show("math-max     ", () => trace((m) => Math.max(m("a"), 0)));
show("math-variadic", () => [Math.max(...[1, 2, 3, 4, 5, 6, 7, 8]), Math.min(3, 1), Math.hypot(3, 4)]);
show("math-bigint  ", () => { try { return Math.max(1n); } catch (e) { return e.constructor.name; } });
show("isNaN        ", () => trace((m) => isNaN(m("a"))));
// `parseInt` stringifies its first argument and `ToInt32`s its radix.
show("parseInt-str ", () => trace((m) => parseInt(m("a"))));
show("parseInt-radx", () => trace((m) => parseInt("11", m("a"))));
show("parseFloat   ", () => trace((m) => parseFloat(m("a"))));
// `arr.length = v` runs BOTH conversions 10.4.2.4 specifies.
show("length-assign", () => trace((m) => { const a = [1, 2, 3]; a.length = m("a"); return a.length; }));
show("length-throws", () => { const a = [1, 2, 3]; try { a.length = { valueOf() { throw new Error("L"); } }; return "no throw"; } catch (e) { return e.message; } });
show("length-bad   ", () => { const a = [1]; try { a.length = 1.5; return "no throw"; } catch (e) { return e.constructor.name; } });
// A throwing conversion propagates out of each family.
show("throws-array ", () => { try { return [1, 2].slice({ valueOf() { throw new Error("A"); } }); } catch (e) { return e.message; } });
show("throws-number", () => { try { return (1).toFixed({ valueOf() { throw new Error("N"); } }); } catch (e) { return e.message; } });
show("throws-math  ", () => { try { return Math.abs({ valueOf() { throw new Error("M"); } }); } catch (e) { return e.message; } });
