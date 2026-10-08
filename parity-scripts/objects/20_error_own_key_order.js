// Own-key order of error instances: V8 captures `stack` first, then the
// constructor installs message, cause and (AggregateError) errors.
const names = (e) => Object.getOwnPropertyNames(e).join(",");
console.log(names(new Error("m")), "|", names(new Error()));
console.log(names(new Error("m", { cause: 1 })));
console.log(names(new AggregateError([])), "|", names(new AggregateError([], "x", { cause: 2 })));
class E extends Error {}
console.log(names(new E("q")));
try { null.x; } catch (e) { console.log(names(e)); }
try { JSON.parse("{"); } catch (e) { console.log(names(e)); }
try { new URL("bad"); } catch (e) { console.log(names(e)); }
try { require("path").join(1); } catch (e) { console.log(names(e)); }
try { require("fs").readFileSync("/nonexistent/zz"); } catch (e) { console.log(names(e)); }
