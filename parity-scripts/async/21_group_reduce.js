// Functional grouping/counting into Map and Object; sorted deterministic output.
const words = ['apple', 'banana', 'cherry', 'avocado', 'blueberry', 'cranberry'];

// Group by first letter into a Map (insertion order follows first-seen).
const byLetter = new Map();
for (const w of words) {
  const key = w[0];
  if (!byLetter.has(key)) byLetter.set(key, []);
  byLetter.get(key).push(w);
}
const grouped = [...byLetter.entries()].map(([k, v]) => k + ':[' + v.join(',') + ']');
console.log('grouped=' + grouped.join(' '));

// Frequency count into a plain object, printed in sorted key order.
const letters = 'mississippi'.split('');
const freq = letters.reduce((acc, ch) => {
  acc[ch] = (acc[ch] || 0) + 1;
  return acc;
}, {});
const sortedFreq = Object.keys(freq).sort().map((k) => k + '=' + freq[k]);
console.log('freq=' + sortedFreq.join(','));

// Partition via reduce into two buckets.
const [evens, odds] = [1, 2, 3, 4, 5, 6, 7].reduce(
  ([e, o], n) => (n % 2 === 0 ? [[...e, n], o] : [e, [...o, n]]),
  [[], []],
);
console.log('evens=' + evens.join(',') + ' odds=' + odds.join(','));

// Object.entries -> sorted -> map
const scores = { bob: 3, ann: 5, cid: 1 };
console.log('ranked=' + Object.entries(scores)
  .sort((a, b) => b[1] - a[1])
  .map(([n, s]) => n + s)
  .join(','));
