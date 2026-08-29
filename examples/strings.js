// String methods, template literals, and the padding/trimming family.
const s = "Hello, World";
console.log(s.toUpperCase(), s.toLowerCase(), s.length);
console.log(s.slice(7), s.slice(-5), s.substring(0, 5), s.at(-1));
console.log(s.indexOf("o"), s.lastIndexOf("o"), s.includes("World"));
console.log(s.startsWith("Hello"), s.endsWith("!"), s.charAt(4));
console.log("  pad  ".trim(), "|", "  pad  ".trimStart(), "|", "  pad  ".trimEnd(), "|");
console.log("7".padStart(3, "0"), "7".padEnd(3, "-"));
console.log("a-b-c".split("-"), [..."abc"], "ab".repeat(3));
console.log(s.replace("World", "there"), s.replaceAll("l", "L"));
console.log("x".concat("y", "z"), String(42), (42).toString(2));
const name = "node", n = 3;
console.log(`${name} has ${n} letters: ${n > 2 ? "many" : "few"}`);
console.log("abc".localeCompare("abd"), "b" > "a", "10" < "9");
console.log(String.fromCharCode(65, 66), "AB".charCodeAt(1));
