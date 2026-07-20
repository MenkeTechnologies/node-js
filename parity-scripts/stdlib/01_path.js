// path: join / resolve-relative / basename / dirname / extname / sep.
const path = require("path");

console.log(path.join("/foo", "bar", "baz/asdf", "quux", ".."));
console.log(path.join("a", "b", "..", "c", "./d"));
console.log(path.normalize("/foo/bar//baz/asdf/quux/.."));
console.log(path.basename("/foo/bar/baz/asdf/quux.html"));
console.log(path.basename("/foo/bar/baz/asdf/quux.html", ".html"));
console.log(path.dirname("/foo/bar/baz/asdf/quux"));
console.log(path.extname("index.coffee.md"));
console.log(path.extname("index."));
console.log(path.extname("index"));
console.log(path.relative("/data/orandea/test/aaa", "/data/orandea/impl/bbb"));
console.log(path.sep === "/" ? "posix-sep" : "other-sep");
console.log(path.posix.join("/a", "b", "c"));
