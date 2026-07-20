// sort with comparator, reverse, slice, splice, concat, fill, copyWithin.
const src = [3, 1, 4, 1, 5, 9, 2, 6];

console.log('sortAsc=' + [...src].sort((a, b) => a - b).join(','));
console.log('sortDesc=' + [...src].sort((a, b) => b - a).join(','));
console.log('sortLex=' + [...src].sort().join(','));
console.log('reverse=' + [...src].reverse().join(','));
console.log('slice=' + src.slice(2, 5).join(','));
console.log('sliceNeg=' + src.slice(-3).join(','));

const spliced = [1, 2, 3, 4, 5];
const removed = spliced.splice(1, 2, 'a', 'b', 'c');
console.log('splice-removed=' + removed.join(','));
console.log('splice-after=' + spliced.join(','));

console.log('concat=' + [1, 2].concat([3, 4], 5).join(','));
console.log('fill=' + new Array(4).fill(7).join(','));
console.log('fillRange=' + [1, 2, 3, 4, 5].fill(0, 1, 3).join(','));
console.log('copyWithin=' + [1, 2, 3, 4, 5].copyWithin(0, 3).join(','));
