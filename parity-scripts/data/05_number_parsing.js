// parseInt radix, parseFloat, Number() coercion.
console.log(parseInt("ff", 16));
console.log(parseInt("0x1A"));          // auto-hex
console.log(parseInt("777", 8));
console.log(parseInt("1010", 2));
console.log(parseInt("42abc"));         // trailing junk
console.log(parseInt("   -13   "));     // whitespace
console.log(parseInt("z", 36));
console.log(parseInt("abc"));           // NaN

console.log(parseFloat("3.14xyz"));
console.log(parseFloat("  6.022e23 "));
console.log(parseFloat(".5"));
console.log(parseFloat("Infinity"));
console.log(parseFloat("1e-3"));

console.log(Number("42"));
console.log(Number("  3.14  "));
console.log(Number(""));                // 0
console.log(Number("0x10"));            // 16
console.log(Number("0b101"));           // 5
console.log(Number("0o17"));            // 15
console.log(Number("1e3"));
console.log(Number("abc"));             // NaN
console.log(Number(true), Number(false), Number(null));
console.log(Number([]), Number([5]), Number([1, 2]));
