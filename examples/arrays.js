// Array methods and higher-order functions.
const nums = [5, 3, 8, 1, 9, 2];
console.log(nums.filter(x => x % 2 === 1).map(x => x * x));
console.log(nums.reduce((a, b) => a + b, 0));
console.log([...nums].sort((a, b) => a - b));
console.log(nums.slice(1, 4), nums.indexOf(8), nums.includes(9));
const [first, second, ...rest] = nums;
console.log(first, second, rest);
