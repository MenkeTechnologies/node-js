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

{
// V8's two stack-capture knobs. Both were absent: `Error.stackTraceLimit` read
// `undefined` (so the common save-and-restore idiom installed `undefined` and
// disabled the limit permanently) and `Error.prepareStackTrace` read
// `undefined` too, sending every `if (Error.prepareStackTrace)` probe down the
// wrong branch.
console.log("defaults", typeof Error.stackTraceLimit, Error.stackTraceLimit,
  typeof Error.prepareStackTrace, Error.prepareStackTrace.name);

// The limit is honoured, not merely reported: it caps the frames a stack keeps,
// and 0 is the documented way to make error construction cheap.
function outer() { return middle(); }
function middle() { return inner(); }
function inner() { return new Error("deep"); }
const withLimit = (n, f) => {
  const saved = Error.stackTraceLimit;
  Error.stackTraceLimit = n;
  try { return f(); } finally { Error.stackTraceLimit = saved; }
};
console.log("limits  ", withLimit(0, () => outer().stack.split("\n").length),
  withLimit(1, () => outer().stack.split("\n").length),
  withLimit(2, () => new Error("m").stack.split("\n").length));
console.log("settable", withLimit(5, () => Error.stackTraceLimit), Error.stackTraceLimit);

// A custom `prepareStackTrace` replaces `.stack` entirely — the hook every
// source-map library installs. It was honoured only by `captureStackTrace`, so
// an ordinary `err.stack` read bypassed it.
const withHook = (hook, f) => {
  const saved = Error.prepareStackTrace;
  Error.prepareStackTrace = hook;
  try { return f(); } finally { Error.prepareStackTrace = saved; }
};
console.log("hook    ", withHook(() => "CUSTOM", () => new Error("x").stack));
console.log("hookArgs", withHook((e, sites) => `${e.message}/${Array.isArray(sites)}`,
  () => new Error("y").stack));
// Restoring it puts the default back.
console.log("restored", new Error("z").stack.split("\n")[0]);

// The default hook is a real function and renders the header plus one line per
// call site, so a library may call it directly on a stack it captured.
const dflt = Error.prepareStackTrace;
console.log("default ", dflt(new Error("m"), []), JSON.stringify(dflt(new Error(), [])));
// `captureStackTrace` still works, and goes through whatever hook is installed.
const captured = {};
Error.captureStackTrace(captured);
console.log("capture ", typeof captured.stack,
  withHook(() => "VIA-HOOK", () => { const o = {}; Error.captureStackTrace(o); return o.stack; }));

// The ordinary stack is unchanged: header, then indented frames.
const plain = new Error("boom");
console.log("plain   ", plain.stack.split("\n")[0], plain.stack.includes("    at "));
}
