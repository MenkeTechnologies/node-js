// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: microtask
process.on("uncaughtException",e=>console.log("h",e.message)); queueMicrotask(()=>{throw new Error("q")}); Promise.resolve().then(()=>console.log("then"))
