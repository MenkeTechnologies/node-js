// The v (unicodeSets) flag: nested classes, -- and && set operators, u/v
// exclusivity; and JS empty classes [] / [^].
const t=(n,f)=>{let r;try{r=f()}catch(e){r="THROW "+e.constructor.name+": "+e.message}console.log(n.padEnd(10),JSON.stringify(r))};
t("basic", ()=>[/a/v.test("a"), /a/v.unicodeSets, /a/v.unicode, /a/v.flags, String(/x/gv), new RegExp("a","v").source]);
t("sub", ()=>"abcdef".match(/[[a-z]--[aeiou]]/gv));
t("inter", ()=>"a1b2".match(/[\w&&\d]/gv));
t("nested", ()=>"xa-".match(/[[a-c][x-z]]/gv));
t("prop", ()=>"héllo1".match(/[\p{L}--[a-z]]/gv));
t("emoji", ()=>"😀a".match(/./gv));
t("uv", ()=>new RegExp("a","uv"));
t("vv", ()=>new RegExp("a","vv"));
t("neg", ()=>"abc1".match(/[^\d]/gv));
t("empty", ()=>[/[]/v.test("a"), /[^]/v.test("a")]);
console.log(/[]/.test("a"), /[^]/.test("\n"), /a[]b/.test("ab"), "a\nb".match(/a[^]b/)[0].length, /[]a]/.test("a]"), "x]".replace(/[^]]/g, "-"), /[\]]/.test("]"), /[^]*/.exec("abc")[0]);
