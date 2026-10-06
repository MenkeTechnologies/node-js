// Error.captureStackTrace: the stack header comes from the target's name and
// message, the property is non-enumerable, and a primitive target throws.
const o={}; Error.captureStackTrace(o); console.log(o.stack.split("\n")[0], Object.keys(o));
const p={name:"N", message:"m"}; Error.captureStackTrace(p); console.log(p.stack.split("\n")[0]);
const q={message:"only"}; Error.captureStackTrace(q); console.log(q.stack.split("\n")[0]);
const r={name:"OnlyName"}; Error.captureStackTrace(r); console.log(r.stack.split("\n")[0]);
class E extends Error{}; const e=new E("z"); Error.captureStackTrace(e); console.log(e.stack.split("\n")[0]);
const f=function(){}; Error.captureStackTrace(f); console.log(f.stack.split("\n")[0]);
try { Error.captureStackTrace(5) } catch (x) { console.log(x.message) }
const a={name:"",message:"mm"}; Error.captureStackTrace(a); console.log(a.stack.split("\n")[0]);
const b={name:"",message:""}; Error.captureStackTrace(b); console.log(JSON.stringify(b.stack.split("\n")[0]));
