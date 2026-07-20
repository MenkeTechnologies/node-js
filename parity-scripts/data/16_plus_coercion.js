// The `+` operator: string concat vs numeric add coercion.
console.log(1 + 2);
console.log("1" + 2);
console.log(1 + "2");
console.log("a" + 1 + 2);
console.log(1 + 2 + "a");
console.log(1 + null);          // 1
console.log(1 + undefined);     // NaN
console.log("x" + null);        // "xnull"
console.log("x" + undefined);   // "xundefined"
console.log(true + 1);          // 2
console.log(true + true);       // 2
console.log([] + []);           // ""
console.log([] + {});           // "[object Object]"
console.log([1, 2] + [3, 4]);   // "1,23,4"
console.log(1 + true + "3");    // "23"
console.log("" + 42);
console.log("" + null);
console.log("" + undefined);
console.log("" + true);
console.log("" + [1, 2, 3]);
console.log("" + {});
console.log(+"42", +"3.14", +"", +"abc"); // unary plus
console.log(3 + 4 * 2);
console.log("5" - 2, "5" * 2, "5" / 2);   // non-+ arithmetic coerces
