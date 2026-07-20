// typeof of every value kind.
console.log(typeof 42);
console.log(typeof 3.14);
console.log(typeof NaN);
console.log(typeof Infinity);
console.log(typeof "str");
console.log(typeof true);
console.log(typeof undefined);
console.log(typeof null);          // "object" (historical)
console.log(typeof {});
console.log(typeof []);            // "object"
console.log(typeof function () {});
console.log(typeof (() => {}));
console.log(typeof 1n);            // "bigint"
console.log(typeof Symbol());      // "symbol"
console.log(typeof Symbol.iterator);
console.log(typeof /regex/);       // "object"
console.log(typeof new Date(0));   // "object"; Date(0) is deterministic
console.log(typeof Math);
console.log(typeof JSON);
console.log(typeof parseInt);
console.log(typeof class C {});    // "function"
console.log(typeof (typeof 1));    // "string"

const kinds = [0, "", true, null, undefined, [], {}, () => {}, 1n];
console.log(kinds.map((v) => typeof v).join(","));
