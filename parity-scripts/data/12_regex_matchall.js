// matchAll and named-group iteration (supported subset).
const text = "id=1 id=22 id=333";
for (const m of text.matchAll(/id=(\d+)/g)) {
  console.log(m[0], "->", m[1], "@", m.index);
}

const kv = "a:1,b:2,c:3";
const pairs = [...kv.matchAll(/(?<key>\w):(?<val>\d)/g)];
console.log(pairs.map((p) => `${p.groups.key}=${p.groups.val}`).join(" "));

const words = [..."the quick brown fox".matchAll(/\w+/g)];
console.log(words.length, words.map((w) => w[0].length).join(","));

const nums = [..."10 20 30 40".matchAll(/\d+/g)].map((m) => Number(m[0]));
console.log(nums.reduce((a, b) => a + b, 0));

// count matches
console.log([..."aXbXcXd".matchAll(/X/g)].length);

// group with alternation
for (const m of "cat3 dog7 fish9".matchAll(/([a-z]+)(\d)/g)) {
  console.log(m[1], m[2]);
}
