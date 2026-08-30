// Iterator helpers (27.1.4). None of them existed: `[1,2,3].values().map(f)`
// was "map is not a function". They are LAZY, which is the whole point — a
// `take(3)` on an endless generator has to stop after three pulls rather than
// materialise anything, so each stage holds the iterator it draws from and
// only the terminal operations drain.
function* g() { yield 1; yield 2; yield 3; yield 4; }
const t = (l, f) => { try { const r = f(); console.log(l, typeof r === "string" ? r : JSON.stringify(r)); } catch (e) { console.log(l, e.constructor.name + ": " + e.message); } };

console.log("global  ", typeof Iterator, typeof Iterator.from, typeof Iterator.prototype.take);
console.log("lazy    ", g().take(2).toArray().join(","), g().drop(2).toArray().join(","),
  g().map((x) => x * 2).toArray().join(","), g().filter((x) => x % 2 === 0).toArray().join(","));
console.log("flatMap ", g().flatMap((x) => [x, -x]).toArray().join(","));
console.log("chain   ", g().map((x) => x * 3).filter((x) => x % 2 === 1).take(2).toArray().join(","));
// Terminal operations.
console.log("reduce  ", g().reduce((a, b) => a + b), g().reduce((a, b) => a + b, 10));
console.log("search  ", g().some((x) => x > 3), g().some((x) => x > 9),
  g().every((x) => x > 0), g().every((x) => x > 1), g().find((x) => x > 2), g().find((x) => x > 9));
console.log("forEach ", (() => { const o = []; g().forEach((x) => o.push(x)); return o.join(","); })());

// Laziness is observable: an endless source is pulled exactly as far as needed.
function* naturals() { let i = 0; while (true) { yield i++; } }
console.log("endless ", naturals().filter((x) => x % 3 === 0).take(4).toArray().join(","),
  naturals().drop(5).take(2).toArray().join(","));
let pulled = 0;
function* counted() { while (true) { pulled++; yield pulled; } }
console.log("pulls   ", counted().take(3).toArray().join(","), pulled);

// Every iterator carries them, not just generators.
console.log("builtins", [1, 2, 3].values().map((x) => x + 1).toArray().join(","),
  [1, 2, 3].keys().map((i) => i * 2).toArray().join(","),
  "hello"[Symbol.iterator]().filter((c) => c !== "l").toArray().join(""),
  new Set([1, 2, 3]).values().drop(1).toArray().join(","),
  new Map([["a", 1]]).entries().toArray().flat().join(","));
console.log("entries ", JSON.stringify([...[10, 20].entries().map(([i, v]) => i + v)]));

// A helper IS an iterator, so it spreads, for-ofs and destructures.
console.log("iterable", [...g().map((x) => x * 2)].join(","), Array.from(g().take(2)).join(","),
  (() => { const [a, b] = g().map((x) => x * 10); return `${a},${b}`; })(),
  g().take(2)[Symbol.iterator]() !== undefined);
console.log("brand   ", Object.prototype.toString.call(g().take(1)),
  Object.getPrototypeOf(g().take(1)) === Object.getPrototypeOf(g().map((x) => x)));

// `Iterator.from` accepts an iterable OR a bare `next`-bearing object, and what
// it returns carries the helpers.
console.log("from    ", Iterator.from([1, 2, 3]).take(2).toArray().join(","));
console.log("from-obj", Iterator.from({ next: (() => { let i = 0; return () => ({ value: i, done: i++ > 1 }); })() }).toArray().join(","));

// Abandoning a chain CLOSES the iterator underneath it, so a generator's
// `finally` runs and the source cannot be drained again.
let closed = 0;
function* closing() { try { yield 1; yield 2; yield 3; } finally { closed++; } }
closing().take(1).toArray();
console.log("closed  ", closed);
const chain = g().map((x) => x);
console.log("drained ", chain.take(1).toArray().join(","), "|" + chain.toArray().join(",") + "|");
const stopped = closing().map((x) => x);
stopped.next();
stopped.return();
console.log("returned", closed, JSON.stringify(stopped.next()));
// A helper that has finished stays finished.
const spent = g().take(1);
spent.toArray();
console.log("spent   ", JSON.stringify(spent.next()));

// Argument validation: a limit must be a non-negative number, and a callback
// must be callable.
t("neg     ", () => g().take(-1));
t("nan     ", () => g().take(NaN));
t("nofn    ", () => g().map());
t("notfn   ", () => g().filter(5));
t("empty   ", () => [g().take(0).toArray().length, g().drop(9).toArray().length].join(","));
t("noseed  ", () => { function* e() {} return e().reduce((a, b) => a + b); });
t("seed    ", () => { function* e() {} return e().reduce((a, b) => a + b, 7); });
