// try/catch/finally ordering, custom Error subclasses, and what a thrown
// non-Error carries through.
class MyError extends Error {
  constructor(msg, code) { super(msg); this.name = "MyError"; this.code = code; }
}
try { throw new MyError("boom", 42); }
catch (e) { console.log(e.name, e.message, e.code, e instanceof MyError, e instanceof Error); }
function order() {
  const seen = [];
  try { seen.push("try"); throw new Error("x"); }
  catch { seen.push("catch"); return seen.join(","); }
  finally { seen.push("finally"); }
}
console.log(order());
function finallyWins() { try { return "try"; } finally { console.log("cleanup"); } }
console.log(finallyWins());
try { throw "a string"; } catch (e) { console.log(typeof e, e); }
try { null.x; } catch (e) { console.log(e.constructor.name); }
try { undefinedFn(); } catch (e) { console.log(e.constructor.name); }
try { JSON.parse("["); } catch (e) { console.log(e instanceof SyntaxError); }
console.log(new Error("m").message, String(new TypeError("t")));
const nested = () => { try { try { throw new Error("inner"); } finally { console.log("inner-finally"); } } catch (e) { return e.message; } };
console.log(nested());

// `.stack`'s header line is formatted on the FIRST READ, from whatever `name`
// and `message` the error carries at that moment — not from what the `Error`
// constructor was called with. Building it eagerly inside `super()` meant the
// near-universal subclass-that-renames-itself reported `Error:`.
const head = (e) => e.stack.split("\n")[0];
class Renamed extends Error { constructor(m) { super(m); this.name = "Renamed"; } }
console.log(head(new Renamed("boom")));
const late = new Error("boom"); late.name = "Late"; late.message = "changed";
console.log(head(late));
// A `name` inherited from the prototype counts too.
class ProtoNamed extends Error {}
ProtoNamed.prototype.name = "ProtoNamed";
console.log(head(new ProtoNamed("boom")));
// Formatted exactly once: a rename AFTER the first read changes nothing.
const settled = new Error("boom"); const firstRead = head(settled); settled.name = "TooLate";
console.log(firstRead, head(settled));
// An explicit assignment wins permanently over that lazy formatting.
const assigned = new Error("x"); assigned.stack = "CUSTOM";
console.log(assigned.stack);
// The slots stay own-but-non-enumerable, so keys/JSON see none of them.
const plain = new Error("x");
console.log(Object.keys(plain).length, JSON.stringify(plain), Object.prototype.hasOwnProperty.call(plain, "stack"));
