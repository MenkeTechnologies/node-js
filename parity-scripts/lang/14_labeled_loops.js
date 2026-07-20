// Labeled loops: break and continue with labels.
const grid = [
  [1, 2, 3],
  [4, 5, 6],
  [7, 8, 9],
];

let found = null;
outer: for (let i = 0; i < grid.length; i++) {
  for (let j = 0; j < grid[i].length; j++) {
    if (grid[i][j] === 5) {
      found = [i, j];
      break outer;
    }
  }
}
console.log("found 5 at:", found.join(","));

// continue with label to skip whole row.
const collected = [];
rows: for (let i = 0; i < grid.length; i++) {
  for (let j = 0; j < grid[i].length; j++) {
    if (grid[i][j] % 2 === 0) continue rows; // skip row on first even
    collected.push(grid[i][j]);
  }
}
console.log("collected:", collected.join(","));

// Labeled block with break.
let sum = 0;
compute: {
  sum = 10;
  if (sum > 5) break compute;
  sum = 999; // unreachable
}
console.log("sum:", sum);

// Nested labels.
let pairs = [];
loopA: for (const a of [1, 2, 3]) {
  loopB: for (const b of [1, 2, 3]) {
    if (a === b) continue loopB;
    if (a + b > 4) continue loopA;
    pairs.push(`${a}-${b}`);
  }
}
console.log("pairs:", pairs.join(","));
