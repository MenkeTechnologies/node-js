// `ToPropertyKey` and the remaining `ToNumber` argument sites. A property key
// given as an OBJECT was used without conversion — `({k: 7}).hasOwnProperty(o)`
// was false and `Object.defineProperty(obj, o, …)` created a property literally
// named `[object Object]` — while the plain read/write/delete forms coerced
// correctly, so the same key behaved differently depending on which operation
// asked about it.
//
// Nothing here uses a LOCAL-time Date method: this frontend treats local time
// as UTC (see BUGS.md), which would make the result depend on the machine.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const trace = (call) => {
  const log = [];
  const mk = (tag) => ({ valueOf() { log.push(tag + ":valueOf"); return 1; }, toString() { log.push(tag + ":toString"); return "k"; } });
  let out;
  try { out = call(mk); } catch (e) { out = e.constructor.name; }
  return [log, out];
};

// The forms that already coerced, for contrast with the ones that did not.
show("read         ", () => trace((m) => ({ k: 7 })[m("a")]));
show("write        ", () => trace((m) => { const o = {}; o[m("a")] = 1; return Object.keys(o); }));
show("delete       ", () => trace((m) => { const o = { k: 1 }; delete o[m("a")]; return Object.keys(o); }));
// …and the ones that did not.
show("in           ", () => trace((m) => m("a") in { k: 1 }));
show("hasOwnProperty", () => trace((m) => ({ k: 1 }).hasOwnProperty(m("a"))));
show("Object.hasOwn", () => trace((m) => Object.hasOwn({ k: 1 }, m("a"))));
show("defineProperty", () => trace((m) => { const o = {}; Object.defineProperty(o, m("a"), { value: 1, configurable: true }); return Object.getOwnPropertyNames(o); }));
show("getOwnPropDesc", () => trace((m) => Object.getOwnPropertyDescriptor({ k: 1 }, m("a"))));
show("Reflect.get  ", () => trace((m) => Reflect.get({ k: 7 }, m("a"))));
show("Reflect.has  ", () => trace((m) => Reflect.has({ k: 1 }, m("a"))));
show("Reflect.delete", () => trace((m) => { const o = { k: 1 }; Reflect.deleteProperty(o, m("a")); return Object.keys(o); }));
// A SYMBOL key is a key, not something to stringify, and a throwing conversion
// propagates.
show("symbol-key   ", () => { const s = Symbol("s"); const o = { [s]: 1 }; return [o[s], s in o, o.hasOwnProperty(s), Object.getOwnPropertyDescriptor(o, s).value]; });
show("throws       ", () => { try { return ({ k: 1 }).hasOwnProperty({ toString() { throw new Error("K"); } }); } catch (e) { return e.message; } });
// `Date` setters coerce every argument (UTC forms, so the answer does not
// depend on the machine's zone).
show("setUTCFullYr ", () => trace((m) => { const d = new Date(0); d.setUTCFullYear(m("a")); return d.getUTCFullYear(); }));
show("setUTCMonth  ", () => trace((m) => { const d = new Date(0); d.setUTCMonth(m("a")); return d.getUTCMonth(); }));
show("setTime      ", () => trace((m) => { const d = new Date(0); d.setTime(m("a")); return d.getTime(); }));
show("date-throws  ", () => { const d = new Date(0); try { d.setTime({ valueOf() { throw new Error("D"); } }); return "no throw"; } catch (e) { return e.message; } });
// `LengthOfArrayLike` is `ToLength(Get(O, 'length'))`, which coerces too.
show("Array.from   ", () => trace((m) => Array.from({ length: m("a") }).length));
show("array-like-0 ", () => Array.from({ length: { valueOf: () => 0 } }).length);
