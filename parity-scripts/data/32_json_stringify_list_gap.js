// JSON.stringify: the replacer-array PropertyList (String/Number elements and
// their wrappers only, deduplicated, read through a Proxy) and the gap (wrapper
// unwrapping, ten-unit string cap, V8 numeric gap where 0 < n < 1 still breaks lines).
const o = { a: 1, b: { a: 2, c: 3 }, 1: "one", c: [{ a: 4, z: 5 }] };
console.log(JSON.stringify(o, ["a", "a", 1, new String("b"), new Number(1), true, null, {}, Symbol("a"), "c"]));
console.log(JSON.stringify(o, null, "abcdefghijklmnop"));
console.log(JSON.stringify(o, null, 15.7), JSON.stringify([1], null, -3), JSON.stringify([1], null, new Number(2)), JSON.stringify([1], null, new String("xy")), JSON.stringify([1], null, true));
console.log(JSON.stringify({ a: 1 }, null, 0.9), JSON.stringify([1, 2], null, ""), JSON.stringify([1], null, 1e100));
const arrLike = new Proxy(["a"], {}); console.log(JSON.stringify({ a: 1, b: 2 }, arrLike));
let reads = [];
const traced = new Proxy(["b", "a"], { get(t, k, r) { reads.push(String(k)); return Reflect.get(t, k, r); } });
console.log(JSON.stringify({ a: 1, b: 2 }, traced), reads);
console.log(JSON.stringify({ 1.5: "x", 2: "y" }, [1.5, 2, 1e21]), JSON.stringify({ a: 1 }, null, NaN), JSON.stringify({ a: 1 }, null, -Infinity), JSON.stringify({ a: [] }, null, Infinity));
console.log(JSON.stringify({ a: 1 }, null, Object.assign(new Number(3), { valueOf() { return 1; } })), JSON.stringify({ a: 1 }, null, Object.assign(new String("ab"), { toString() { return "Q"; } })));
