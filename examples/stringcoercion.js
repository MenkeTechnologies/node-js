// `String.prototype` coerces every argument it is handed (22.1.3.x): a numeric
// position through `ToNumber`, every other through `ToString`. Neither ran —
// `'x'.padStart({valueOf: () => 3})` produced `"x"` and
// `'x'.concat({toString: () => 'y'})` produced `"x[object Object]"`, with the
// user's method never called. Twenty-two positions measured, all wrong.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
// Each probe reports which conversion ran, so a coercion that produces the
// right answer by luck is still visible.
const trace = (call) => {
  const log = [];
  const mk = (tag) => ({ valueOf() { log.push(tag + ":valueOf"); return 1; }, toString() { log.push(tag + ":toString"); return "b"; } });
  let out;
  try { out = call(mk); } catch (e) { out = e.constructor.name; }
  return [log, out];
};

// String positions run `toString`…
show("concat       ", () => trace((m) => "x".concat(m("a"))));
show("startsWith   ", () => trace((m) => "abc".startsWith(m("a"))));
show("includes     ", () => trace((m) => "abc".includes(m("a"))));
show("indexOf      ", () => trace((m) => "abc".indexOf(m("a"))));
show("lastIndexOf  ", () => trace((m) => "abc".lastIndexOf(m("a"))));
show("padStart-pad ", () => trace((m) => "x".padStart(3, m("a"))));
show("split-sep    ", () => trace((m) => "a-b".split(m("a"))));
show("replace-find ", () => trace((m) => "abc".replace(m("a"), "z")));
show("localeCompare", () => trace((m) => "abc".localeCompare(m("a"))));
// …and numeric positions run `valueOf`, in the same methods.
show("padStart-len ", () => trace((m) => "x".padStart(m("a"))));
show("indexOf-from ", () => trace((m) => "abc".indexOf("b", m("a"))));
show("split-limit  ", () => trace((m) => "a-b".split("-", m("a"))));
show("at           ", () => trace((m) => "abc".at(m("a"))));
show("charAt       ", () => trace((m) => "abc".charAt(m("a"))));
show("charCodeAt   ", () => trace((m) => "abc".charCodeAt(m("a"))));
show("repeat       ", () => trace((m) => "ab".repeat(m("a"))));
show("slice        ", () => trace((m) => "abcd".slice(m("a"))));
show("substring    ", () => trace((m) => "abcd".substring(m("a"))));
// A throwing conversion propagates, and `valueOf` is preferred for a numeric
// position while `toString` is for a string one — the same object, both ways.
show("throws       ", () => { try { return "x".padStart({ valueOf() { throw new Error("boom"); } }); } catch (e) { return e.message; } });
show("prefers-num  ", () => "x".padStart({ valueOf: () => 3, toString: () => "9" }));
show("prefers-str  ", () => [1, 2].join({ valueOf: () => 9, toString: () => "-" }));
// The positions that must NOT be stringified keep their own path: a RegExp, a
// `Symbol.replace`/`split` carrier, and a callable replacement.
show("regexp-split ", () => "a1b2c".split(/\d/));
show("callable-repl", () => "abc".replace("b", (m) => m.toUpperCase()));
show("split-protocol", () => "abc".split({ [Symbol.split]() { return ["X"]; } }));
show("repl-protocol", () => "abc".replace({ [Symbol.replace]() { return "Y"; } }, "z"));
// `IsRegExp` tests `Symbol.match` for TRUTHINESS, so a falsy one coerces
// normally while a truthy one is rejected.
show("isregexp-true", () => { try { return "abc".startsWith({ [Symbol.match]: true }); } catch (e) { return e.constructor.name; } });
show("isregexp-fals", () => "abc".includes({ [Symbol.match]: false, toString: () => "b" }));
show("regexp-arg   ", () => { try { return "abc".includes(/a/); } catch (e) { return e.constructor.name; } });
// A non-RegExp argument to `match`/`matchAll`/`search` is turned INTO one, so a
// metacharacter matches as one. `match` answered `null` for every string —
// indistinguishable from a real miss — and `search` did a substring scan.
show("match-string ", () => "abc".match("b"));
show("match-meta   ", () => "a.c".match("."));
show("match-absent ", () => "abc".match());
show("matchAll     ", () => [..."aXbXc".matchAll("X")].map((m) => m.index));
show("search-meta  ", () => "a.c".search("."));
show("search-absent", () => "abc".search());
show("search-miss  ", () => "abc".search("z"));
