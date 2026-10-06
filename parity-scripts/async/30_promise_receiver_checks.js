// Promise executor and static-method receiver checks.
for (const v of [5, undefined, null, {}, [], [1,2], "s", Symbol("q"), 1n, new Map(), /r/, true]) { try { new Promise(v) } catch (e) { console.log(e.message) } }
for (const m of ["resolve","reject","all","any","race","allSettled","withResolvers","try"]) { try { Promise[m].call(1, []) ; console.log(m, "ok") } catch (e) { console.log(m, e.message) } } for (const m of ["resolve","all"]) { try { Promise[m].call(undefined, []) } catch (e) { console.log(m, e.message) } try { Promise[m].call({}, []) } catch (e) { console.log(m, e.message) } }
