// util.inspect.custom: depth/options/inspect arguments, string vs value answers,
// newline re-indentation, self-return, throws, Map/Set entries, %s/%o/%O.
const util=require("util");
class A{[util.inspect.custom](d,o,i){ return `A<${d}>` }}
class B{constructor(){this.v=[1,2]} [util.inspect.custom](d,o,i){ return i(this.v, o) + "|" + typeof o.stylize + "|" + Object.keys(o).join(",") }}
class C{[util.inspect.custom](){ return {replaced:true, n:new A()} }}
class M{[util.inspect.custom](){ return "line1\nline2" }}
console.log(new A(), {a:new A(), deep:{x:{y:{z:new A()}}}});
console.log(new B());
console.log(new C(), [new C()]);
console.log({m:new M(), arr:[new M()]});
console.log(util.inspect(new A(), {depth:5}), util.inspect(new A(), {customInspect:false}));
console.log(util.format("%s %o %O %j", new A(), new A(), new A(), new A()));
const self={[util.inspect.custom](){ return this }, x:1}; console.log(self);
const thrower={[util.inspect.custom](){ throw new Error("ins") }}; try{console.log(thrower)}catch(e){console.log("caught", e.message)}
console.log(String(util.inspect.custom), util.inspect.custom === Symbol.for("nodejs.util.inspect.custom"));
const plain={ [Symbol.for("nodejs.util.inspect.custom")]: () => "viaFor" }; console.log(plain, `${util.inspect([plain])}`);
console.log(new Map([[new A(), new A()]]), new Set([new A()]));
