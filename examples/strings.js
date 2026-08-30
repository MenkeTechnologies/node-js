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

// GetSubstitution (22.1.3.19) runs for a STRING search value too, not only a
// regexp. The string path did a raw replace and passed the template through
// verbatim, so `'abc'.replace('b', '[$&]')` produced `a[$&]c`.
console.log("sub-amp ", "abc".replace("b", "[$&]"), "abc".replace("b", "[$`]"), "abc".replace("b", "[$']"));
console.log("sub-dd  ", "abc".replace("b", "$$"), "abc".replace("b", "$1"));
// Each match expands against its OWN position, so `$`` differs per occurrence.
console.log("sub-all ", "aba".replaceAll("a", "[$&]"), "aXbXc".replaceAll("X", "($`)"));
// The regexp path was already correct and must stay so.
console.log("re-sub  ", "abc".replace(/b/, "[$&]"), "a1b".replace(/(\d)/, "<$1>"), "x1".replace(/(?<d>\d)/, "<$<d>>"));

// A replacement CALLBACK takes a final `groups` argument when the pattern has
// named groups. Omitting it left `arguments.length` at 4 where node reports 5.
console.log("cb-plain", "a1b".replace(/(\d)/, function () { return arguments.length; }));
console.log("cb-named", "x1".replace(/(?<d>\d)/, function () { return arguments.length; }));
console.log("cb-args ", "x1".replace(/(?<d>\d)/, (...a) => JSON.stringify(a)));
console.log("cb-str  ", "abc".replace("b", function () { return arguments.length; }));

// `replaceAll` cannot honour "all" without `g`, so a non-global regexp is a
// TypeError (22.1.3.20 step 2). It used to replace the first match silently.
try { "aa".replaceAll(/a/, "b"); } catch (e) { console.log("nonglobal", e.constructor.name, e.message); }
console.log("global  ", "aa".replaceAll(/a/g, "b"), "aa".replace(/a/, "b"));
