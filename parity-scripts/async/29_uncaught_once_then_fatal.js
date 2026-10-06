// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: once_then_fatal
process.once("uncaughtException",e=>console.log("h",e.message)); setTimeout(()=>{throw new Error("1")}); setTimeout(()=>{throw new Error("2")},2)
