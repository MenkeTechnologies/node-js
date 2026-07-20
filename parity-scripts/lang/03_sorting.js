// Quicksort and mergesort implementations.
function quicksort(arr) {
  if (arr.length <= 1) return arr;
  const [pivot, ...rest] = arr;
  const left = rest.filter((x) => x < pivot);
  const right = rest.filter((x) => x >= pivot);
  return [...quicksort(left), pivot, ...quicksort(right)];
}

function mergesort(arr) {
  if (arr.length <= 1) return arr;
  const mid = Math.floor(arr.length / 2);
  const left = mergesort(arr.slice(0, mid));
  const right = mergesort(arr.slice(mid));
  const merged = [];
  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (left[i] <= right[j]) merged.push(left[i++]);
    else merged.push(right[j++]);
  }
  return merged.concat(left.slice(i)).concat(right.slice(j));
}

const data = [5, 2, 9, 1, 7, 3, 8, 4, 6, 0];
console.log("quick:", quicksort(data).join(","));
console.log("merge:", mergesort(data).join(","));
console.log("orig:", data.join(",")); // unchanged
