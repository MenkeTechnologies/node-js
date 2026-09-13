// The CommonJS module surface. Three members were missing: `module.parent`,
// `require.extensions` and `require.resolve.paths`.

// `module.parent` is long deprecated but PRESENT, and code still tests
// `if (!module.parent)` to detect "run as the entry point". The key was absent
// entirely, so that test read as `undefined` for the wrong reason. It is NOT
// enumerable — `Object.keys(module)` does not list it.
console.log("parent  ", "parent" in module, module.parent, Object.keys(module).includes("parent"));
console.log("keys    ", JSON.stringify(Object.keys(module)));
console.log("shape   ", typeof module.id, typeof module.filename, typeof module.path,
  Array.isArray(module.paths), Array.isArray(module.children), typeof module.loaded);
console.log("main    ", require.main === module, typeof require.main);

// `require.extensions` — the legacy loader map, deprecated but still read by
// tooling that hooks module loading. It did not exist.
console.log("ext     ", typeof require.extensions,
  JSON.stringify(Object.keys(require.extensions)), typeof require.extensions[".js"]);

// `require.resolve.paths(spec)` — the directories a lookup would search. A core
// module is not searched for on disk at all, so it reports `null`.
console.log("paths   ", require.resolve.paths("fs"), require.resolve.paths("node:fs"));
console.log("relative", Array.isArray(require.resolve.paths("./x")),
  require.resolve.paths("./x").length);
// The `node_modules` chain is reported; node also appends three
// installation-specific global folders this runtime does not search, so the
// LENGTH is deliberately not asserted here.
console.log("package ", Array.isArray(require.resolve.paths("some-package")),
  require.resolve.paths("some-package")[0].endsWith("node_modules"));

// The rest of the surface is unchanged.
console.log("resolve ", require.resolve("fs"), typeof require.cache, typeof require.resolve);
console.log("builtin ", (() => { const m = require("module");
  return [m.isBuiltin("fs"), m.isBuiltin("./x"), m.builtinModules.includes("path"),
    typeof m.createRequire].join(","); })());
console.log("dirs    ", typeof __dirname, typeof __filename, __filename.endsWith(".js"));
console.log("prefix  ", require("node:fs") === require("fs"));
