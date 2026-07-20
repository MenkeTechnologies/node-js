// Template literals & interpolation.
const name = "World";
const n = 42;
console.log(`Hello, ${name}!`);
console.log(`Sum: ${1 + 2 + 3}`);
console.log(`n=${n}, n^2=${n * n}, hex=${n.toString(16)}`);
console.log(`nested ${`inner ${n}`} done`);
console.log(`multi
line
string`);
console.log(`${n > 40 ? "big" : "small"}`);

const items = ["a", "b", "c"];
console.log(`list: ${items.join(", ")}`);
console.log(`len: ${items.length}`);

function tag(strings, ...values) {
  return strings.reduce((acc, str, i) => acc + str + (i < values.length ? `[${values[i]}]` : ""), "");
}
console.log(tag`x=${1} y=${2} z=${3}`);

const obj = { a: 1, b: 2 };
console.log(`obj: ${JSON.stringify(obj)}`);
console.log(`escaped: \${not interpolated}`);
console.log(`tab\tnewline-repr`);
