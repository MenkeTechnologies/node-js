// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: only_monitor_fatal
process.on("uncaughtExceptionMonitor",(e,o)=>console.log("mon",e.message,o)); throw new Error("only monitor")
