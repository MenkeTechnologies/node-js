// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: throw_primitive
process.on("uncaughtException",(e,o)=>console.log("h",String(e),o)); throw 3; console.log("no")
