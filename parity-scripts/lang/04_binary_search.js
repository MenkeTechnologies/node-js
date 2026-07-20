// Iterative and recursive binary search.
function binarySearch(arr, target) {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (arr[mid] === target) return mid;
    if (arr[mid] < target) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

function binarySearchRec(arr, target, lo = 0, hi = arr.length - 1) {
  if (lo > hi) return -1;
  const mid = Math.floor((lo + hi) / 2);
  if (arr[mid] === target) return mid;
  return arr[mid] < target
    ? binarySearchRec(arr, target, mid + 1, hi)
    : binarySearchRec(arr, target, lo, mid - 1);
}

const sorted = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19];
for (const t of [7, 1, 19, 8, 13]) {
  console.log(`iter find ${t}:`, binarySearch(sorted, t));
  console.log(`rec  find ${t}:`, binarySearchRec(sorted, t));
}
