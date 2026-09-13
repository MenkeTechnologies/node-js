// The protocols keyed on the well-known symbols. The symbols themselves now
// exist, but nothing consulted them: a custom matcher was stringified, an
// array-like could not opt into `concat` spreading, and `IsRegExp` looked only
// at the heap representation.

// 22.1.3.x step 2: a string method delegates to a `Symbol.*` method on its
// ARGUMENT rather than coercing it to a pattern.
console.log("match   ", "abc".match({ [Symbol.match](s) { return "matched:" + s; } }));
console.log("search  ", "abc".search({ [Symbol.search](s) { return 42; } }));
console.log("split   ", "abc".split({ [Symbol.split](s) { return ["A", "B"]; } }).join("|"));
console.log("replace ", "abc".replace({ [Symbol.replace](s, r) { return "R:" + r; } }, "x"));
console.log("matchAll", [..."abc".matchAll({ [Symbol.matchAll](s) { return [1, 2][Symbol.iterator](); } })].join(","));
// The extra arguments reach the method: `replace` passes its replacement.
console.log("args    ", "abc".replace({ [Symbol.replace](s, r) { return `${s}/${r}`; } }, "Z"));
// A RegExp still takes the ordinary path.
console.log("regexp  ", "xax".match(/a/)[0], "xax".search(/a/), "xaxa".split(/a/).join("|"),
  "abc".replace(/b/, "X"));
// Its symbol-keyed methods ARE those implementations, reachable on an instance.
console.log("re-syms ", /a/[Symbol.match]("xax")[0], /a/[Symbol.search]("xax"),
  /a/[Symbol.split]("xaxa").join("|"), /b/[Symbol.replace]("abc", "X"));

// 23.1.3.1: `Symbol.isConcatSpreadable` overrides `IsArray` in BOTH directions.
const arrayLike = { length: 2, 0: "a", 1: "b", [Symbol.isConcatSpreadable]: true };
console.log("spread  ", [1].concat(arrayLike).join(","));
const optedOut = [1, 2];
optedOut[Symbol.isConcatSpreadable] = false;
console.log("no-spread", [0].concat(optedOut).length, JSON.stringify([0].concat(optedOut)[1]));
console.log("default ", [1].concat([2, 3]).join(","), [1].concat({ a: 1 }).length);

// 7.2.8 `IsRegExp` asks `Symbol.match` first, so an object can claim the label
// and a real regexp can disown it. These three methods reject a regexp outright.
const reject = (f) => { try { f(); return "no-throw"; } catch (e) { return e.constructor.name; } };
console.log("reject  ", reject(() => "abc".startsWith(/a/)), reject(() => "abc".endsWith(/c/)),
  reject(() => "abc".includes(/b/)));
console.log("claims  ", reject(() => "abc".startsWith({ [Symbol.match]: true })));
const disowned = /a/;
disowned[Symbol.match] = false;
console.log("disowns ", "abc".startsWith(disowned), disowned[Symbol.match]);
console.log("plain   ", "abc".startsWith("a"), "abc".endsWith("c"), "abc".includes("b"));

// 22.1.3.20 step 2.a: `replaceAll` checks the `g` flag BEFORE consulting
// `Symbol.replace`, so a non-global regexp is still a TypeError even though a
// RegExp does define that method.
console.log("replAll ", reject(() => "aa".replaceAll(/a/, "b")), "aa".replaceAll(/a/g, "b"));

// 23.1.3.38: the methods a `with` block must not bring into scope, as a
// null-prototype object.
const unscopables = Array.prototype[Symbol.unscopables];
console.log("unscope ", typeof unscopables, Object.getPrototypeOf(unscopables),
  Object.keys(unscopables).length, unscopables.flat, unscopables.push);
