// The replacer sees every key, with the holder as `this`.
const seen = [];
JSON.stringify({ a: 1, b: { c: 2 } }, function (k, v) {
  seen.push(`${JSON.stringify(k)}@${JSON.stringify(this)}`);
  return v;
});
console.log(seen.join(' | '));
// It TRANSFORMS values, in objects and arrays alike.
console.log(JSON.stringify({ a: 1, b: 2 }, (k, v) => (typeof v === 'number' ? v * 2 : v)));
console.log(JSON.stringify([1, 2], (k, v) => (typeof v === 'number' ? v * 2 : v)));
// Returning undefined drops an object key and nulls an array slot.
console.log(JSON.stringify({ a: 1, b: 2 }, (k, v) => (k === 'b' ? undefined : v)));
console.log(JSON.stringify([1, 2], (k, v) => (k === '0' ? undefined : v)));
// The top-level value is reached under key "" and is replaceable too.
console.log(JSON.stringify(5, (k, v) => v * 2));
// An ARRAY second argument is still the key filter, not a replacer.
console.log(JSON.stringify({ a: 1, b: 2 }, ['a']));
