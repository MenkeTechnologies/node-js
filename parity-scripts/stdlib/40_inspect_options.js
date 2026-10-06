// util.inspect options: numericSeparator, maxStringLength, getters ('get'/'set'),
// showProxy, and a RegExp's own enumerable properties.
const u=require("util"); const o={numericSeparator:true};
console.log([1234, -1234567, 123.4567891, 0.0001234, 1e21, 1.5e-7, -0, 12345.678, 999, 1000, NaN, 1234567.1234567].map(n=>u.inspect(n,o)).join(" "));
console.log(u.inspect([1000000, {a:2500}], o), u.inspect(-12345678901234567890n, o), u.inspect(123n,o));
console.log(u.inspect("abcdef",{maxStringLength:1}), u.inspect(["abcdef","ab"],{maxStringLength:2}), u.inspect("abc",{maxStringLength:0}), u.inspect({k:"x".repeat(50)},{maxStringLength:10}));
console.log(u.inspect({get g(){return {a:1}}, set s(v){}, get gs(){return 1}, set gs(v){}},{getters:true}));
console.log(u.inspect({get g(){return 1}, get gs(){return 2}, set gs(v){}},{getters:"get"}), u.inspect({get g(){return 1}, get gs(){return 2}, set gs(v){}},{getters:"set"}));
console.log(u.inspect(Object.assign(/x/g, {y:2})), u.inspect(Object.assign(/a/, {lastIndex:3})), u.inspect(Object.assign(/x/, {o:{p:1}})));
console.log(u.format("%d %i", 1234567, 1234567), u.inspect(new Map([[1,"abcdef"]]),{maxStringLength:2}));
let n=0; const og={get a(){n++; return {deep:{x:{y:1}}}}, get b(){return null}, get c(){return "s"}, get f(){return function q(){}}, get arr(){return [1,2]}};
console.log(u.inspect(og,{getters:true}), n);
console.log(u.inspect({nested:{get z(){return 9}}},{getters:true}), u.inspect({get z(){return 9}}), n);
console.log(u.inspect({get a(){return 1}, set a(v){}, get b(){return 2}},{getters:"set"}));
console.log(u.inspect({p:new Proxy([1],{})},{showProxy:true,depth:0}), u.inspect(new Proxy(new Proxy({},{}),{}),{showProxy:true}), u.inspect(new Proxy({a:1},{})));
