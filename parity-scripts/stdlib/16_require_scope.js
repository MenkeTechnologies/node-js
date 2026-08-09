// A required module's body must not see the locals of whatever function called
// `require`, and `vm.runInThisContext` must not see them either.
//
// The module wrapper and the `vm` code are compiled at RUNTIME, from inside the
// calling function, so a nested run that simply executed on the current frame
// picked up that frame's scope. Both probes below evaluate a name that exists
// ONLY as a caller local, so an implementation that runs nested source in the
// caller's frame reports its type instead of `undefined`.
const fs = require("fs");
const os = require("os");
const path = require("path");
const vm = require("vm");

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "njs-reqscope-"));
const mod = path.join(dir, "probe.js");
fs.writeFileSync(mod, "module.exports = typeof secret + ',' + typeof alsoHidden;\n");

function load() {
  const secret = 1;
  var alsoHidden = 2;
  void secret;
  void alsoHidden;
  return require(mod);
}
console.log("module body sees:", load());

function viaVm() {
  const secret = 1;
  void secret;
  return vm.runInThisContext("typeof secret");
}
console.log("runInThisContext sees:", viaVm());

function viaScript() {
  const secret = 1;
  void secret;
  return new vm.Script("typeof secret").runInThisContext();
}
console.log("vm.Script sees:", viaScript());

// The module is still cached by path and still gets the real wrapper arguments.
console.log("cached identical:", load() === load());

fs.unlinkSync(mod);
fs.rmdirSync(dir);
