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
