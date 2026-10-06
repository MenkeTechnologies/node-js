// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: handler_throws_exit7
console.log("start"); process.on("exit",c=>console.log("exit",c)); process.on("uncaughtException",(e,o)=>{throw new Error("again")}); throw new Error("u")
