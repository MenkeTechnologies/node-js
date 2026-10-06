// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: reference_error
process.on("uncaughtException",(e,o)=>console.log("h",e instanceof TypeError, e.message)); undefinedFn()
