// Word frequency counting with objects and Math.
const text = "the quick brown fox the lazy dog the fox";
const counts = {};
for (const word of text.split(" ")) {
  counts[word] = (counts[word] || 0) + 1;
}
const entries = Object.entries(counts).sort((a, b) => b[1] - a[1]);
for (const [word, n] of entries) {
  console.log(`${word}: ${n}`);
}
console.log("max frequency:", Math.max(...Object.values(counts)));
