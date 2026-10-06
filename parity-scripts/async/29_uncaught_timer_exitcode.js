// process.on('uncaughtException') / 'uncaughtExceptionMonitor' delivery: timer_exitcode
process.on("uncaughtException",(e,o)=>{console.log("h",e.message); process.exitCode=2}); process.on("exit",c=>console.log("exit",c)); setTimeout(()=>{throw new Error("a")}); setTimeout(()=>console.log("b"),2)
