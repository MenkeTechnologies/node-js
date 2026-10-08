// RegExp.prototype.test and toString are generic (22.2.6.16-17): test runs
// RegExpExec — a callable `exec` read with [[Get]] is called and must return an
// object or null — and toString reads `source` and `flags`. exec and compile
// brand-check their receiver.
const t = (s) => {
  try {
    console.log(s, require("util").inspect(eval(s)));
  } catch (e) {
    console.log(s, e.constructor.name + ": " + e.message);
  }
};
for (const s of [
  "RegExp.prototype.exec.call(null)",
  "RegExp.prototype.test.call(null)",
  "RegExp.prototype.toString.call(1)",
  "RegExp.prototype.test.call({})",
  "RegExp.prototype.test.call({exec:1})",
  "RegExp.prototype.compile.call({})",
  "RegExp.prototype.test.call({exec(s){console.log('exec', typeof s, s); return null}}, {toString(){return 'q'}})",
  "RegExp.prototype.toString.call({get source(){console.log('src'); return 'a'}, get flags(){console.log('fl'); return 'g'}})",
  "RegExp.prototype.toString.call(RegExp.prototype)",
  "RegExp.prototype.test.call(/a/, {toString(){return 'a'}})",
  "RegExp.prototype.exec.call(/a/, {toString(){return 'xa'}}).index",
  "RegExp.prototype.test.call({exec(){return {}}}, 'a')",
  "RegExp.prototype.test.call({exec(){return 1}}, 'a')",
  "(()=>{const r=/a/; r.exec=()=>null; return r.test('a')})()",
  "(()=>{const r=/a/; r.exec=()=>({}); return r.test('zz')})()",
  "/a/.test()",
  "/undefined/.test()",
  "String(/a\\/b/g)",
  "RegExp.prototype.toString.call({source:'x', flags:'q'})",
  "(()=>{const r=/a/g; return [r.test('aa'), r.lastIndex, r.test('aa'), r.lastIndex, r.test('aa'), r.lastIndex]})()",
])
  t(s);
const r = /a/;
RegExp.prototype.exec = function (s) {
  console.log("patched exec", s);
  return null;
};
t("r.test('a')");
