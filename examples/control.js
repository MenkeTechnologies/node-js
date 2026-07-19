// Control flow: loops, switch, try/catch.
let total = 0;
for (const v of [1, 2, 3, 4, 5]) {
  if (v === 3) continue;
  total += v;
}
console.log("total", total);

function classify(x) {
  switch (true) {
    case x < 0: return "negative";
    case x === 0: return "zero";
    default: return "positive";
  }
}
console.log(classify(-4), classify(0), classify(7));

try {
  throw new Error("failure");
} catch (e) {
  console.log("caught:", e.message);
} finally {
  console.log("done");
}
