// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: monitor_order
process.on("uncaughtExceptionMonitor",(e,o)=>console.log("mon",e.message,o)); process.on("uncaughtException",e=>console.log("h",e.message)); throw new Error("m")
