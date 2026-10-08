// A failed `require` reports node's require stack (Module._resolveFilename):
// the message lists the requiring module and each module that first loaded the
// one before it, and the error carries `code` then `requireStack`.
const fs = require("fs");
const os = require("os");
const path = require("path");

const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "njs-reqstack-")));
const scrub = (s) => String(s).split(dir).join("<tmp>").split(__filename).join("<entry>");
const show = (e) => {
  console.log(scrub(e.message));
  console.log(e.code, scrub(JSON.stringify(e.requireStack)), Object.keys(e));
};
fs.writeFileSync(path.join(dir, "outer.js"), "module.exports = () => require('./inner');\n");
fs.writeFileSync(
  path.join(dir, "inner.js"),
  "try { require('./missing'); } catch (e) { module.exports = e; }\n",
);

try { require("./definitely-missing"); } catch (e) { show(e); }
try { require("no-such-package-anywhere"); } catch (e) { show(e); }
try { require.resolve("./nope"); } catch (e) { show(e); }
show(require(path.join(dir, "outer.js"))());
const created = require("module").createRequire(path.join(dir, "virtual.js"));
try { created("./gone"); } catch (e) { show(e); }
try { created.resolve("./gone"); } catch (e) { show(e); }

for (const f of ["outer.js", "inner.js"]) fs.unlinkSync(path.join(dir, f));
fs.rmdirSync(dir);
