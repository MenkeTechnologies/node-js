// == vs === coercion tables.
const looseCases = [
  [1, "1"], [0, false], [0, ""], ["", false], [null, undefined],
  [1, true], [0, "0"], ["1", true], [null, 0], [undefined, 0],
  [NaN, NaN], ["abc", "abc"], [0, "0.0"], [" \t\n", 0],
];
for (const [a, b] of looseCases) {
  console.log(`${JSON.stringify(a)} == ${JSON.stringify(b)} : ${a == b} | === : ${a === b}`);
}

console.log("--- strict-only ---");
console.log(1 === 1, 1 === "1", null === null, undefined === undefined);
console.log(null == undefined, null === undefined);
console.log(NaN == NaN, NaN === NaN);

// object identity
const o = {};
console.log(o == o, o === o, {} == {});
console.log([] == false, [] == "", [0] == false);
