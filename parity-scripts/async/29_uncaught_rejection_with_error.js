// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: rejection_with_error
process.on("uncaughtException",(e,o)=>console.log("h",e.message,o)); Promise.reject(new TypeError("tt")); console.log("sync")
