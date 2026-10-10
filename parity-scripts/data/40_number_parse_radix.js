// parseInt past 2^53 in radixes 2/4/8/16/32 rounds half-to-even exactly; the other radixes use V8's
// 32-bit chunked accumulation, which is only approximately the true value.
const s = ['9007199254740993', '123456789012345678901234567890', 'zzzzzzzzzzzzzzzzzzzz', 'ffffffffffffffffffffff',
  '0x1fffffffffffff1', '0x20000000000001', '0x20000000000003', '0b' + '1'.repeat(70), '0o' + '7'.repeat(30)];
for (const x of s) console.log(JSON.stringify(x), Number(x), parseInt(x, 36), parseInt(x, 16), parseInt(x, 8), parseInt(x, 32), parseInt(x, 7), parseInt(x));
// StringNumericLiteral grammar: Rust's float parser accepts more than JS does.
for (const x of ['infinity', 'inf', 'nan', '1e', '.5', '5.', '+.5e1', '-.5', '1_0', '0x', '+0x10', '1e+', '  12\n', 'Infinity', '-Infinity', '+Infinity', '0b102', '0o8'])
  console.log(JSON.stringify(x), Number(x));
console.log(Infinity.toPrecision(-1), NaN.toExponential(200), Infinity.toExponential(-3));
