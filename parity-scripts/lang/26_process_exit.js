// `process.exit()` stops execution immediately.
//
// Node needs no `return` after it, so real code does not write one — and a
// `process.exit` that merely returned `undefined` let the next statement run.
// Every line below would print if it did, and the pending timer and microtask
// would fire too; none of them may appear in the output.
const os = require("os");

// An `exit` listener with an EMPTY body cannot distinguish "fires" from "never
// fires", which is what this line was for two rounds while the event was not
// delivered at all. It prints now, so the file checks both things it is here
// for: that the handler runs, and that it does not resurrect execution.
process.on("exit", (code) => console.log("exit handler", code));
setTimeout(() => console.log("TIMER — must not run"), 0);
Promise.resolve().then(() => console.log("MICROTASK — must not run"));
process.nextTick(() => console.log("TICK — must not run"));

function guard(done) {
  if (done) {
    process.exit(0);
  }
  return "FELL THROUGH — must not appear";
}

console.log("start", typeof os.EOL);
console.log(guard(true));
console.log("AFTER CALL — must not run");
