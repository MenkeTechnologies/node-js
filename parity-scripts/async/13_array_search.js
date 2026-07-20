// Array find/findIndex/findLast/some/every/includes/indexOf/at.
const arr = [10, 20, 30, 40, 50];

console.log('find=' + arr.find((n) => n > 25));
console.log('findIndex=' + arr.findIndex((n) => n > 25));
console.log('findLast=' + arr.findLast((n) => n < 45));
console.log('findLastIndex=' + arr.findLastIndex((n) => n < 45));
console.log('some=' + arr.some((n) => n === 30));
console.log('every=' + arr.every((n) => n >= 10));
console.log('includes=' + arr.includes(40));
console.log('includes-miss=' + arr.includes(99));
console.log('indexOf=' + arr.indexOf(30));
console.log('lastIndexOf=' + [1, 2, 1, 2].lastIndexOf(2));
console.log('at-pos=' + arr.at(1));
console.log('at-neg=' + arr.at(-1));
