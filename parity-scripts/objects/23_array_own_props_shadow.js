// An array's own named properties shadow Array.prototype (OrdinaryGet), are
// definable with Object.defineProperty, and Array.prototype.toString calls
// whatever `join` the receiver resolves (23.1.3.36).
const a = [1, 2];
a.join = () => "J";
a.push = function () { return "P"; };
console.log(typeof a.join, a.join === Array.prototype.join, Object.keys(a), a.hasOwnProperty("join"));
console.log(a.join(), a.push(3), a.length, String(a), `${a}`, a + "");
const b = [1];
b.map = 5;
console.log(b.map, Object.keys(b));
try { b.map(); } catch (e) { console.log(e.constructor.name); }
Object.defineProperty(b, "slice", { value: () => "S" });
console.log(typeof b.slice, Object.getOwnPropertyNames(b), b.hasOwnProperty("slice"), b.slice(), b);
const c = [1, 2];
Object.defineProperty(c, "foo", { value: 3, enumerable: true });
console.log(c.foo, Object.keys(c), c, JSON.stringify(Object.getOwnPropertyDescriptor(c, "foo")));
for (const k in c) console.log("in", k);
console.log(Object.entries(c), { ...c });
const d = [1, 2];
d.join = 5;
console.log(String(d));
const e = [3];
Object.defineProperty(e, "join", { get() { console.log("get join"); return Array.prototype.join; } });
console.log(String(e));
console.log(Array.prototype.toString.call({ join() { return "objjoin"; } }), Array.prototype.toString.call({}), Array.prototype.toString.call("ab"));
class X extends Array { join() { return "sub"; } }
console.log(String(X.from([1])));
