// IteratorNext (7.4.4 step 3): an iterator result that is not an object is a
// TypeError, for for-of, spread, destructuring, the iterable-taking builtins
// and for-await alike. Before, a `next` returning a primitive read `done` as
// undefined forever and the loop never ended.
const t = async (f, l) => {
  try {
    await f();
    console.log(l, "ok");
  } catch (e) {
    console.log(l, e.constructor.name + ": " + e.message);
  }
};
const bad = (r) => ({ [Symbol.iterator]() { return { next() { return r; } }; } });
const badAsync = (r) => ({ [Symbol.asyncIterator]() { return { next() { return r; } }; } });
(async () => {
  for (const r of [1, "s", null, undefined, true]) {
    await t(() => { for (const x of bad(r)) {} }, "for-of " + String(r));
    await t(() => [...bad(r)], "spread " + String(r));
    await t(() => { const [a] = bad(r); }, "destructure " + String(r));
    await t(() => new Set(bad(r)), "Set " + String(r));
    await t(() => Array.from(bad(r)), "Array.from " + String(r));
    await t(async () => { for await (const x of badAsync(r)) {} }, "for-await " + String(r));
    await t(async () => { for await (const x of badAsync(Promise.resolve(r))) {} }, "for-await promise " + String(r));
    await t(async () => { for await (const x of bad(r)) {} }, "for-await sync " + String(r));
  }
  await t(async () => { for await (const x of { [Symbol.asyncIterator]() { return 1; } }) {} }, "async method result");
  await t(() => { for (const x of { [Symbol.iterator]() { return { next() { return { get done() { throw new Error("D"); } }; } }; } }) {} }, "done getter throws");
  await t(async () => {
    let i = 0;
    for await (const x of badAsync({ get done() { return i++ > 1; }, value: "v" })) console.log("got", x);
  }, "object result");
})();
