// EventEmitter: non-Error 'error' payloads wrap in ERR_UNHANDLED_ERROR with
// context; listener and setMaxListeners argument validation.
const {EventEmitter} = require("events");
for (const v of ["str", undefined, 5, {a:1}, null, [1], new TypeError("te"), Symbol("s")]) { const e = new EventEmitter(); try { e.emit("error", v) } catch (x) { console.log(String(x), x.code, x.context === v, x === v) } }
try { new EventEmitter().on("x", 5) } catch (x) { console.log(String(x), x.code) }
try { new EventEmitter().once("x", "s") } catch (x) { console.log(String(x)) }
try { new EventEmitter().prependListener("x", null) } catch (x) { console.log(String(x)) }
try { new EventEmitter().off("x", {}) } catch (x) { console.log(String(x)) }
try { new EventEmitter().removeListener("x", 1) } catch (x) { console.log(String(x)) }
try { new EventEmitter().setMaxListeners(-1) } catch (x) { console.log(String(x)) }
try { new EventEmitter().setMaxListeners("a") } catch (x) { console.log(String(x)) }
