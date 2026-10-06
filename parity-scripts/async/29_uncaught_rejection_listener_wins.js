// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: rejection_listener_wins
process.on("uncaughtException",e=>console.log("h",e.message)); process.on("unhandledRejection",r=>console.log("ur",r)); Promise.reject(1)
