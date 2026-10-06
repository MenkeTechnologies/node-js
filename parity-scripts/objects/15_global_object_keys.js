// The global object lists node's enumerable builtins first, then script-made
// globals in creation order; CommonJS wrapper locals and compiler temporaries
// are not properties of it; `typeof` agrees with the bare read (`crypto`,
// `performance`). navigator/sessionStorage are filtered: not implemented.
var q=1; let r=2; function fz(){} x = 3; globalThis.y = 4;
const ks = Object.keys(globalThis).filter(k => k !== "navigator" && k !== "sessionStorage");
console.log(ks);
for (const k in globalThis) if (k !== "navigator" && k !== "sessionStorage") console.log("in:", k);
console.log(JSON.stringify(Object.entries(globalThis).filter(e => e[0] !== "navigator" && e[0] !== "sessionStorage").map(e=>[e[0], typeof e[1]])));
console.log(Object.values(globalThis).includes(setTimeout), Object.entries(globalThis).find(e=>e[0]==="x"));
console.log(globalThis.__filename, globalThis.module, "exports" in globalThis, Object.hasOwn(globalThis, "require"));
setTimeout = 5; console.log(Object.keys(globalThis).indexOf("setTimeout"), globalThis.setTimeout);
delete globalThis.x; console.log(Object.keys(globalThis).includes("x"));
console.log(typeof performance, typeof crypto, typeof globalThis.performance.now);
globalThis.z=1; Object.defineProperty(globalThis,"z",{enumerable:false}); console.log(Object.keys(globalThis).includes("z"), Object.getOwnPropertyNames(globalThis).includes("z"))
