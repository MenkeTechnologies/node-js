// console.table, ported from node: left-justified cells, Map/Set and their
// iterators (read without consuming), sparse columns in ObjectKeys order, the
// table inspect options, East Asian widths, and the properties validation.
console.table([{ a: 1, b: "x" }, { a: 2, c: true }]);
console.table({ r1: { c: 1 }, r2: { c: 2, d: [1] } }, ["c"]);
console.table([[1, 2], [3]]); console.table(5); console.table("s", ["a"]); console.table(null); console.table(new Map([["k", { v: 1 }]]));
console.table([1, "two", { three: 3 }, [4, 5, 6, 7], null, undefined, () => 8]);
console.table([{ deep: { a: 1, b: 2, c: 3 } }, { deep: { a: [1, 2, 3, 4, 5] } }, { 2: "n", 1: "m", z: 1 }]);
console.table(new Set(["a", { b: 1 }]));
console.table(new Set(["a", "b"]).entries()); console.table(new Map([["k", 1]]).keys()); console.table(new Map([["k", 1]]).values());
const it = new Map([["k", 1], ["j", 2]]).entries(); it.next(); console.table(it); console.log([...it]);
console.table({ "日本": { "名前": "テスト", x: "ab" } });
console.table([{ a: 1 }], ["a", "missing"]); console.table([5, 6], ["a"]); console.table([]); console.table({});
console.table([{ get g() { return "getter"; } }]); console.table(Object.assign(Object.create({ inh: 1 }), { own: { inh2: 2 } }));
console.table(new Uint8Array([1, 2])); console.table([new Uint8Array([9]), Buffer.from("hi")]);
try { console.table([1], "x"); } catch (e) { console.log(e.name, e.code, e.message); }
try { console.table([1], { length: 1 }); } catch (e) { console.log(e.name, e.code, e.message); }
console.group("g"); console.table([{ a: 1 }]); console.groupEnd();
const { Console } = require("console"); const { Writable } = require("stream");
let buf = ""; const c = new Console({ stdout: new Writable({ write(ch, e, cb) { buf += ch; cb(); } }) });
c.table([{ x: "y" }]); console.log(JSON.stringify(buf));
