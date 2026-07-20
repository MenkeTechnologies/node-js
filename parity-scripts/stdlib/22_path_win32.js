// path: win32 namespace — deterministic on any host (backslash semantics).
const path = require("path");

console.log(path.win32.join("C:\\temp", "foo", "bar"));
console.log(path.win32.basename("C:\\temp\\myfile.html"));
console.log(path.win32.dirname("C:\\temp\\foo\\bar"));
console.log(path.win32.extname("C:\\temp\\file.txt"));
console.log(path.win32.normalize("C:\\temp\\\\foo\\..\\bar"));
console.log(path.win32.isAbsolute("C:\\foo"));
console.log(path.win32.isAbsolute("foo\\bar"));
console.log(JSON.stringify(path.win32.parse("C:\\path\\dir\\file.txt")));
console.log(path.win32.sep);
console.log(path.win32.delimiter);
console.log(path.posix.delimiter);
console.log(path.win32.relative("C:\\a\\b", "C:\\a\\c\\d"));
