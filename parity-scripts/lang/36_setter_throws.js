// A setter that throws propagates out of the assignment.
const o={set x(v){throw new Error("s")}}; try { o.x=1; console.log("no") } catch (e) { console.log("threw", e.message) } class A { set y(v) { throw new RangeError("r") } } try { new A().y = 2; console.log("no") } catch (e) { console.log("threw", e.name) }
