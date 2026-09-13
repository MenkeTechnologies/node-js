// `util.types` carried six predicates that were hardcoded `false` because the
// thing they test for "did not exist in node-js". Each of those claims had
// since stopped being true — the module doc still asserted "there is no `Proxy`"
// and "no boxed primitives exist" — so the predicates quietly reported the
// wrong answer for values the runtime had gained.
const T = require("util").types;

console.log("proxy   ", T.isProxy(new Proxy({}, {})), T.isProxy(new Proxy([], {})), T.isProxy({}));
// Wrapper objects: the predicate reads the boxed primitive, not the shape.
console.log("boxed   ", T.isBoxedPrimitive(new Number(1)), T.isBoxedPrimitive(new String("a")),
  T.isBoxedPrimitive(1), T.isBoxedPrimitive("a"));
console.log("boxkind ", T.isNumberObject(new Number(1)), T.isStringObject(new String("a")),
  T.isBooleanObject(new Boolean(1)), T.isSymbolObject(Object(Symbol("s"))));
console.log("boxneg  ", T.isNumberObject(1), T.isStringObject("a"), T.isBooleanObject(true),
  T.isNumberObject(new String("a")));
// `DataView` and the two BigInt-backed typed arrays exist now.
console.log("views   ", T.isDataView(new DataView(new ArrayBuffer(1))), T.isDataView(new Uint8Array(1)),
  T.isBigInt64Array(new BigInt64Array(1)), T.isBigUint64Array(new BigUint64Array(1)));
// `isArrayBufferView` covers a DataView as well as every typed array.
console.log("bufview ", T.isArrayBufferView(new Uint8Array(1)),
  T.isArrayBufferView(new DataView(new ArrayBuffer(1))), T.isArrayBufferView(new ArrayBuffer(1)),
  T.isArrayBufferView([]));
// The predicates that were already right stay right.
console.log("unchanged", T.isDate(new Date()), T.isRegExp(/a/), T.isMap(new Map()),
  T.isPromise(Promise.resolve()), T.isGeneratorObject((function* () {})()));

// `arguments` was a plain Array, so `Array.isArray(arguments)` was true, it
// branded as `[object Array]`, and `arguments.map` was a function. Node's is an
// exotic with none of those. It is marked now — still array-BACKED, which is
// what keeps indices, `length`, spread and `for-of` working.
function shape() {
  return [Object.prototype.toString.call(arguments), Array.isArray(arguments),
    T.isArgumentsObject(arguments), typeof arguments.map, typeof arguments.slice].join("|");
}
console.log("args    ", shape(1, 2));
function usable() {
  return [arguments.length, arguments[0], [...arguments].join(","),
    Array.from(arguments).join(","), Array.prototype.slice.call(arguments).join(","),
    Object.keys(arguments).join(",")].join("|");
}
console.log("usable  ", usable(7, 8));
function iterates() { for (const x of arguments) { return x; } }
console.log("iterate ", iterates(9), T.isArgumentsObject([]), T.isArgumentsObject({}));

// `util.promisify.custom` is the registered symbol a module attaches to a
// callback function to supply its own promisified form; it read `undefined`, so
// the lookup that decides whether to use one always missed.
const util = require("util");
console.log("custom  ", typeof util.promisify.custom,
  util.promisify.custom === Symbol.for("nodejs.util.promisify.custom"),
  String(util.promisify.custom));
