// Integer & bitwise ops: ToInt32/ToUint32, shifts, literals.
console.log(0xff, 0o17, 0b1010);
console.log(5 & 3, 5 | 3, 5 ^ 3, ~5);
console.log(1 << 30, 1 << 31, 1 << 32);   // ToInt32 wrap
console.log(-8 >> 1, -8 >>> 1);            // arithmetic vs logical
console.log((-1 >>> 0));                    // 4294967295
console.log((0xffffffff | 0));              // -1 (ToInt32)
console.log((0x100000000 | 0));             // 0 (mod 2^32)
console.log((2 ** 32 + 5) | 0);             // 5
console.log(3.9 | 0, -3.9 | 0);             // truncation toward zero
console.log(NaN | 0, Infinity | 0);         // 0, 0
console.log((255 >>> 4).toString(2));
console.log(0b1111 & 0b0101);
console.log((1 << 10) - 1);
console.log(0xDEADBEEF >>> 0);
console.log((-2147483648 >> 0));
