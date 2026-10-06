// The callee text a failed call reports, as V8's CallPrinter renders each
// expression shape: literals, operators, arrays, templates, assignments, and
// the shapes it leaves as (intermediate value).
{
const o = { a: {} }; let n = 1, s = "s";
const tests = [
  () => (() => 1)()(), () => (function () {})()(), () => (function named() {}).x(), () => (class {}).x(), () => (class K {}).x(),
  () => `x`(), () => `a${n}b`(), () => [1, n, "q"][0](), () => [o][0].a(), () => [, 1][1](), () => [...[1]][0](),
  () => (-n)(), () => (!n)(), () => (typeof n)(), () => (n + 1)(), () => (n ? 1 : 2)(), () => (n = 2)(), () => (n++)(),
  () => /re/g(), () => new Date(0).x(), () => (new Date(0)).getTime.x(), () => o.a.b.c(), () => (o?.a).b(), () => o.a[n](), () => o.a[s + 1](),
  () => o.a[`k`](), () => o.a[o](), () => o.a[1.5](), () => o.a[-1](), () => this.x(), () => (0, n)(), () => (n, o.a.q)(), () => o.a["has space"](), () => o.a[null](),
  () => o.a[true](), () => (n && o)(), () => (n || o.z)(), () => (n ?? o)(), () => (void 0)(), () => (n instanceof Object)(), () => ({ ...o })(), () => ({ a })(),
];
for (const t of tests) { try { t(); console.log("ok"); } catch (e) { console.log(e.message); } }
}
{
const o = { a: {} }; let n = 1;
const tests = [() => o.a["5"](), () => o.a[""](), () => (++n)(), () => (n--)(), () => (+n)(), () => (~n)(), () => (n ** 2)(), () => (n >>> 1)(), () => (n === 1)(), () => ("a" in o)(),
  () => (delete o.q)(), () => [-1][0](), () => (-1)(), () => (- n)(), () => [1n][0](), () => (1n)(), () => [null, undefined, true][0](), () => (n ? o : o)(), () => (n ? n : n)(),
  () => (o.a = 1)(), () => (n += 1)(), () => [function () {}][0].x(), () => [() => 1][0].x(), () => o.a[() => 1](), () => o[[1, 2]](), () => o[{}](), () => o[{ a: 1 }](),
  () => (async () => {})().x(), () => (n, () => 1)(), () => [o.a.b][0](), () => o.a[o.a.b](), () => (1.5e300)(), () => (0.1)(), () => o[`t${n}`](), () => (`t`).x(), () => [`a`][0]()];
for (const t of tests) { try { t(); console.log("ok"); } catch (e) { console.log(e.message); } }
}
