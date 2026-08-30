// `timers/promises.setInterval` and symbol-keyed method calls. Neither had any
// coverage, and the second turned out to be broken for every object.
const tp = require("timers/promises");

// `obj[Symbol.iterator]()` — a symbol-keyed method reached through COMPUTED
// access — reported "is not a function" on every object, a plain literal
// included. The key was being converted with ToString, which renders a symbol
// as `Symbol(Symbol.iterator)`, rather than ToPropertyKey, which produces the
// internal name the method is actually registered under. Reading the same
// property without calling it worked, which is what hid it.
const literal = { [Symbol.iterator]() { return [][Symbol.iterator](); } };
console.log("literal ", typeof literal[Symbol.iterator]());
console.log("builtins", typeof new Map([[1, 2]])[Symbol.iterator](), typeof new Set()[Symbol.iterator]());
console.log("more    ", typeof [][Symbol.iterator](), typeof ""[Symbol.iterator](), typeof new URLSearchParams("a=1")[Symbol.iterator]());
// A user symbol, and the non-symbol computed forms that already worked.
const own = Symbol("own");
console.log("custom  ", ({ [own]: () => "called" })[own]());
console.log("other   ", [() => "idx"][0](), ({ k: () => "str" })["k"](), ({ ab: () => "cat" })["a" + "b"](), ({ 1: () => "num" })[1]());

// `timers/promises.setInterval(delay, value)` is an async ITERABLE — it yields
// `value` every `delay` for as long as it is iterated — and it did not exist.
(async () => {
  const ticks = [];
  for await (const v of tp.setInterval(1, "tick")) {
    ticks.push(v);
    if (ticks.length === 3) break;
  }
  console.log("forawait", ticks.join(","), ticks.length);

  // The iterator protocol by hand. `return()` is what `break` above performs,
  // and a `next()` after it reports done rather than waiting another interval.
  const it = tp.setInterval(1, "m");
  const first = await it.next();
  console.log("next    ", first.value, first.done);
  const closed = await it.return();
  console.log("return  ", String(closed.value), closed.done);
  const after = await it.next();
  console.log("after   ", String(after.value), after.done);
  // It is its own async iterator, as node's is.
  console.log("self    ", typeof it[Symbol.asyncIterator], it[Symbol.asyncIterator]() === it);

  // Called with no value it yields undefined. Stopped afterwards so neither
  // engine is left with a live timer holding the process open.
  const bare = tp.setInterval(1);
  console.log("novalue ", String((await bare.next()).value));
  await bare.return();
})();
