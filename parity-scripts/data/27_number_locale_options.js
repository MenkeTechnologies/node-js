// Number/BigInt toLocaleString with an options bag, en-US: digit options,
// grouping, sign display and the percent/currency styles. ICU rounds the
// SHORTEST decimal half away from zero, so 1.005 rounds up here (unlike toFixed).
const f = (n, o) => { try { return JSON.stringify(n.toLocaleString('en-US', o)); } catch (e) { return e.name + ': ' + e.message; } };
console.log(f(1.005, { maximumFractionDigits: 2 }), f(1.015, { maximumFractionDigits: 2 }), f(2.5, { maximumFractionDigits: 0 }), f(-2.5, { maximumFractionDigits: 0 }), f(0.125, { maximumFractionDigits: 2 }));
console.log(f(1.5, { minimumFractionDigits: 2 }), f(1.23456, { minimumFractionDigits: 2 }), f(1, { minimumFractionDigits: 5 }), f(1, { minimumFractionDigits: 2, maximumFractionDigits: 1 }));
console.log(f(5, { minimumIntegerDigits: 3 }), f(12345.6, { useGrouping: false }), f(-0.0001, {}), f(-0.0001, { maximumFractionDigits: 2 }));
console.log(f(0.256, { style: 'percent' }), f(0.2567, { style: 'percent', minimumFractionDigits: 1 }), f(-1.5, { style: 'percent' }));
console.log(f(1234.5, { style: 'currency', currency: 'USD' }), f(-1234.5, { style: 'currency', currency: 'usd' }), f(1, { style: 'currency', currency: 'EUR' }), f(1, { style: 'currency', currency: 'JPY' }), f(1, { style: 'currency', currency: 'XYZ' }), f(1, { style: 'currency' }));
console.log(f(1, { style: 'currency', currency: 'USD', maximumFractionDigits: 0 }), f(1.239, { style: 'currency', currency: 'USD', minimumFractionDigits: 0 }));
console.log(f(123.456, { maximumSignificantDigits: 2 }), f(0.00123, { maximumSignificantDigits: 1 }), f(5, { minimumSignificantDigits: 3 }), f(1e21, { maximumFractionDigits: 0 }), f(NaN, { style: 'percent' }), f(Infinity, { style: 'currency', currency: 'USD' }));
console.log(f(1, { maximumFractionDigits: 101 }), f(1, { minimumIntegerDigits: 0 }), f(1, { style: 'bogus' }), f(1, { useGrouping: 'always' }), f(1234, { useGrouping: 'min2' }), f(12345, { useGrouping: 'min2' }));
console.log(f(1, { minimumFractionDigits: '2' }), f(1, { minimumFractionDigits: 2.9 }), f(1, { minimumFractionDigits: NaN }), f(1.23456, { minimumSignificantDigits: 2, maximumSignificantDigits: 1 }));
console.log(f(1234.5678, { maximumSignificantDigits: 3 }), f(0.0001234, { minimumSignificantDigits: 5 }), f(1234.5, { minimumFractionDigits: 2, maximumSignificantDigits: 2 }), f(12345678, { maximumSignificantDigits: 2 }));
console.log(f(1, { style: 'currency', currency: 'US' }), f(1, { currency: 'US1' }), f(-1, { style: 'currency', currency: 'eur', currencyDisplay: 'code' }), f(1, { currencyDisplay: 'bad' }), f(1, { signDisplay: 'bad' }));
for (const s of ['auto', 'always', 'exceptZero', 'negative', 'never']) {
  console.log(s, [1, -1, 0, -0, NaN, -Infinity].map((x) => f(x, { signDisplay: s })).join(' '), f(-1, { signDisplay: s, style: 'currency', currency: 'USD' }), f(-0.0001, { signDisplay: s }));
}
for (const c of ['GBP', 'CNY', 'INR', 'CAD', 'KRW', 'CHF', 'XAF', 'XOF', 'BHD', 'CLP', 'XCG']) console.log(c, f(-1234.5, { style: 'currency', currency: c }));
console.log((1234.5).toLocaleString(undefined, { minimumFractionDigits: 2 }), (1234.5).toLocaleString([], { maximumFractionDigits: 0 }), (12n).toLocaleString('en-US', { minimumFractionDigits: 2 }));
console.log(f(1, null), f(1, 5), f(1.45, { maximumFractionDigits: 1 }), f(1.55, { maximumFractionDigits: 1 }), f(8.345, { maximumFractionDigits: 2 }));
for (const x of [0, -0, 1, -1, 0.5, 1.0005, 1.0004999, 12345.678, 1234.5678, 1e21, 1.5e21, 1e100, 123456789012345680000, 5e-324, -1e-10, 0.1+0.2, 999.9995, 2**53, NaN, Infinity, -Infinity, 1/3, 2/3, 1e-7, 4.35, 8.345])
  console.log(String(x), x.toLocaleString(), x.toLocaleString(undefined, {maximumSignificantDigits: 3}), x.toLocaleString('en-US', {style: 'percent', maximumFractionDigits: 1}));
for (const b of [0n, -0n, 123n, -1234567n, 10n ** 30n]) console.log(String(b), b.toLocaleString(), b.toLocaleString('en-US', {style:'currency', currency:'EUR'}), b.toLocaleString(undefined, {maximumSignificantDigits: 2}));
