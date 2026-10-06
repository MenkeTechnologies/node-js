// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: unhandled_rejection_wrapper
process.on("uncaughtException",(e,o)=>console.log("h",e.name,e.code,o,Object.keys(e))); Promise.reject(5); setTimeout(()=>console.log("t"),1)
