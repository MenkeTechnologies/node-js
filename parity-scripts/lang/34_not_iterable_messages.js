// Non-iterable arguments to builtins (Set, Iterator.from, Array.from) and to
// spread, in V8's per-site wording; an @@iterator returning a non-object.
const vals = [["5",5],["-1.5",-1.5],["NaN",NaN],["{}",{}],["true",true],["false",false],["sym",Symbol("s")],["2n",2n],["null",null],["undef",undefined],["fn",function f(){}],["date",new Date(0)],["it1",{[Symbol.iterator]:1}],["itfn",{[Symbol.iterator]:()=>1}],["itnull",{[Symbol.iterator]:null}]];
const cases = { set: v => new Set(v), iterfrom: v => Iterator.from(v), arrfrom: v => Array.from(v), spread: v => [...v] };
for (const [n, f] of Object.entries(cases)) for (const [l, v] of vals) { try { f(v); console.log(n, l, "ok") } catch (e) { console.log(n, l, e.constructor.name, e.message) } }
