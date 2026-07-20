// Core string methods: slice/substring/substr/indexOf/at.
const s = "Hello, World!";
console.log(s.slice(0, 5));
console.log(s.slice(-6));
console.log(s.slice(-6, -1));
console.log(s.substring(7, 12));
console.log(s.substring(12, 7));        // args swapped
console.log(s.substr(7, 5));
console.log(s.indexOf("o"), s.lastIndexOf("o"));
console.log(s.indexOf("z"));
console.log(s.at(0), s.at(-1));
console.log(s.charAt(1), s.charCodeAt(1));
console.log(String.fromCharCode(72, 105));
console.log(s.toUpperCase(), s.toLowerCase());
console.log(s.includes("World"), s.startsWith("Hell"), s.endsWith("!"));
console.log("  trim me  ".trim());
console.log("  left".trimStart(), "right  ".trimEnd());
console.log("aaa".replace("a", "b"));
console.log(s.length);
console.log(s.split("").reverse().join(""));
console.log(s.codePointAt(0));
