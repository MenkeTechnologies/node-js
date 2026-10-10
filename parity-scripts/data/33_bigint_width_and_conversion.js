// BigInt.asIntN / asUintN: `bits` goes through ToIndex, the operand through
// ToBigInt; a width at or past the operand's own size is the identity; and
// BigInt(x) is ToPrimitive first.
const P = (f) => { try { const v = f(); return typeof v === "bigint" ? v + "n" : String(v); } catch (e) { return e.name + ": " + e.message; } };
const bits = [0, 1, 3, 8, 53, 64, 65, 100, -1, 2 ** 53, 2 ** 53 - 1, 2 ** 40, 1.9, "8", NaN, undefined, Infinity];
const vals = [0n, 1n, -1n, 255n, 256n, -129n, 2n ** 64n, -(2n ** 63n), 2n ** 100n - 1n, -(2n ** 100n), 5, "12", true, null, undefined, "0x1f", " 7 ", "1.5", Object(3n), [], [7], {}];
for (const b of bits) {
  console.log(String(b), vals.map((v) => P(() => BigInt.asUintN(b, v))).join(" "));
  console.log(String(b), vals.map((v) => P(() => BigInt.asIntN(b, v))).join(" "));
}
for (const v of [[7], [], Object(3n), {}, { valueOf() { return 5; } }, { valueOf() { return 1.5; } }, new Date(5), Symbol("s"), null, new String("12"), Object(true), 1e21, 1.5, "0b101"]) {
  console.log(P(() => BigInt(v)));
}
