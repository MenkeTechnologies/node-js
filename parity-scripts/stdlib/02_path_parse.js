// path: parse / format round-trip and posix specifics.
const path = require("path");

const parsed = path.parse("/home/user/dir/file.txt");
console.log(JSON.stringify(parsed));
console.log(parsed.root, parsed.dir, parsed.base, parsed.ext, parsed.name);

const formatted = path.format({
  root: "/",
  dir: "/home/user/dir",
  base: "file.txt",
});
console.log(formatted);

console.log(path.format({ dir: "/a/b", name: "index", ext: ".html" }));
console.log(path.isAbsolute("/foo/bar"));
console.log(path.isAbsolute("qux/"));
console.log(JSON.stringify(path.posix.parse("/a/b/c.js")));
