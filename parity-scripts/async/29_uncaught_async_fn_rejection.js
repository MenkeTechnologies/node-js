// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: async_fn_rejection
process.on("uncaughtException",e=>console.log("h",e.message)); async function f(){throw new Error("af")} f(); console.log("x")
