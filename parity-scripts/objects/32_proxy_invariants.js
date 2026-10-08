// Proxy trap results are checked against the target (ECMA-262 10.5.x) with
// V8's wording: a pinned property cannot be misreported by get/set/has/
// deleteProperty/getOwnPropertyDescriptor/defineProperty, ownKeys yields
// property keys without duplicates, preventExtensions/setPrototypeOf/
// isExtensible must agree with the target, construct must return an object,
// and IsArray refuses a revoked proxy. A getOwnPropertyDescriptor result is
// the completed descriptor, and an existing property is redefined as { value }.
const t = (s) => {
  try {
    console.log(s.slice(0, 90), "=>", require("util").inspect(eval(s)));
  } catch (e) {
    console.log(s.slice(0, 90), "=>", e.constructor.name + ": " + e.message);
  }
};
const nc=(o,k,d)=>Object.defineProperty(o,k,d);
for (const s of [
"new Proxy(nc({},'x',{value:1}),{get(){return 2}}).x",
"new Proxy(nc({},'x',{set(v){}}),{get(){return 2}}).x",
"(()=>{'use strict'; new Proxy(nc({},'x',{get(){return 1}}),{set(){return true}}).x=3})()",
"new Proxy({},{ownKeys(){return ['a','a']}}) && Reflect.ownKeys(new Proxy({},{ownKeys(){return ['a','a']}}))",
"Reflect.ownKeys(new Proxy({},{ownKeys(){return [1]}}))",
"Reflect.ownKeys(new Proxy({},{ownKeys(){return [{}]}}))",
"Reflect.ownKeys(new Proxy({},{ownKeys(){return 1}}))",
"Object.isExtensible(new Proxy({},{isExtensible(){return false}}))",
"Object.isExtensible(new Proxy(Object.preventExtensions({}),{isExtensible(){return true}}))",
"Object.preventExtensions(new Proxy({},{preventExtensions(){return true}}))",
"Object.preventExtensions(new Proxy({},{preventExtensions(){return false}}))",
"Reflect.preventExtensions(new Proxy({},{preventExtensions(){return false}}))",
"Object.defineProperty(new Proxy({},{defineProperty(){return true}}),'x',{value:1,configurable:false})",
"Object.defineProperty(new Proxy({},{defineProperty(){return false}}),'x',{value:1})",
"Reflect.defineProperty(new Proxy({},{defineProperty(){return false}}),'x',{value:1})",
"Object.defineProperty(new Proxy(nc({},'x',{value:1,configurable:true,writable:true}),{defineProperty(){return true}}),'x',{value:1,configurable:false})",
"Object.defineProperty(new Proxy(nc({},'x',{value:1,writable:true}),{defineProperty(){return true}}),'x',{value:1,writable:false})",
"Object.defineProperty(new Proxy(nc({},'x',{value:1}),{defineProperty(){return true}}),'x',{value:2})",
"Object.getOwnPropertyDescriptor(new Proxy({},{getOwnPropertyDescriptor(){return {value:1,configurable:false}}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy({},{getOwnPropertyDescriptor(){return 1}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy(Object.preventExtensions({x:1}),{getOwnPropertyDescriptor(){return undefined}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy(Object.preventExtensions({}),{getOwnPropertyDescriptor(){return {value:1,configurable:true}}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy(nc({},'x',{value:1,writable:true}),{getOwnPropertyDescriptor(){return {value:1,configurable:false,writable:false}}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy(nc({},'x',{value:1}),{getOwnPropertyDescriptor(){return {value:2,configurable:false}}}),'x')",
"Object.getOwnPropertyDescriptor(new Proxy({x:1},{getOwnPropertyDescriptor(){return {value:2,configurable:true, enumerable:true, writable:true}}}),'x')",
"new (new Proxy(function(){},{construct(){return 1}}))()",
"Array.isArray(Proxy.revocable([],{}).proxy) && (()=>{const r=Proxy.revocable([],{}); r.revoke(); return Array.isArray(r.proxy)})()",
"Object.setPrototypeOf(new Proxy({},{setPrototypeOf(){return false}}), null)",
"Reflect.setPrototypeOf(new Proxy({},{setPrototypeOf(){return false}}), null)",
"Object.setPrototypeOf(new Proxy(Object.preventExtensions({}),{setPrototypeOf(){return true}}), null)",
"delete new Proxy(Object.preventExtensions({x:1}),{deleteProperty(){return true}}).x",
"(()=>{'use strict'; return delete new Proxy({x:1},{deleteProperty(){return false}}).x})()",
"'x' in new Proxy(Object.preventExtensions({x:1}),{has(){return false}})",
"new Proxy(function(){},{apply(){return 7}})()",
"typeof new Proxy(class{}, {})",
"Object.keys(new Proxy({a:1,b:2},{getOwnPropertyDescriptor(t,k){return k==='a'?undefined:Reflect.getOwnPropertyDescriptor(t,k)}}))",
]) t(s);
const log = [];
const h = {};
for (const k of ["get", "set", "has", "deleteProperty", "ownKeys", "getOwnPropertyDescriptor", "defineProperty", "preventExtensions", "isExtensible", "getPrototypeOf", "setPrototypeOf"])
  h[k] = (...a) => { log.push(k); return Reflect[k](...a); };
for (const s of [
  "(()=>{const p=new Proxy({a:1},h); Object.freeze(p); return [Object.isFrozen(p), log.splice(0).join()]})()",
  "(()=>{const p=new Proxy({a:1},h); Object.seal(p); return [Object.isSealed(p), log.splice(0).join()]})()",
  "(()=>{const p=new Proxy({},h); Object.defineProperty(p,'x',{get(){return 1}, enumerable:true}); return [p.x, Object.keys(p), log.splice(0).join()]})()",
  "(()=>{const p=new Proxy({},h); p.q=1; delete p.q; return ['q' in p, log.splice(0).join()]})()",
  "(()=>{const p=new Proxy([],h); p.push(1,2); return [p.length, log.splice(0).join()]})()",
  "(()=>{const p=new Proxy({},h); Object.setPrototypeOf(p, Array.prototype); return [p instanceof Array, log.splice(0).join()]})()",
  "(()=>{const p=new Proxy({},{preventExtensions(t){Object.preventExtensions(t); return true}}); return [Reflect.preventExtensions(p), Object.isExtensible(p)]})()",
  "Object.freeze(new Proxy({a:1},{preventExtensions(){return false}}))",
  "JSON.stringify({a:1}, new Proxy(['a'],{}))",
  "[].concat(new Proxy([1,2],{}))",
  "(()=>{const r=Proxy.revocable([],{}); r.revoke(); return [].concat(r.proxy)})()",
])
  t(s);
