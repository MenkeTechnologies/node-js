// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: sync_throw
process.on("uncaughtException",(e,o)=>{console.log("h",e.message,o); throw new Error("again")}); throw new Error("u")
