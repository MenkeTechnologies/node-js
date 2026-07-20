// Array map/filter/reduce/reduceRight/flatMap/flat end-to-end.
const nums = [1, 2, 3, 4, 5, 6];

console.log('map=' + nums.map((n) => n * n).join(','));
console.log('filter=' + nums.filter((n) => n % 2 === 0).join(','));
console.log('reduce=' + nums.reduce((a, b) => a + b, 0));
console.log('reduceRight=' + nums.reduceRight((a, b) => a + '-' + b));

const flatMapped = [1, 2, 3].flatMap((n) => [n, n * 10]);
console.log('flatMap=' + flatMapped.join(','));

const nested = [1, [2, [3, [4]]]];
console.log('flat1=' + nested.flat().join(','));
console.log('flatInf=' + nested.flat(Infinity).join(','));

const words = ['aa', 'b', 'ccc'];
console.log('reduceObj=' + JSON.stringify(
  words.reduce((acc, w) => { acc[w] = w.length; return acc; }, {}),
));
