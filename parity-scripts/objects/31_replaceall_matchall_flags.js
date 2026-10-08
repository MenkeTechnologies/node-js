// replaceAll / matchAll hold any IsRegExp argument to a `g` in its `flags`,
// read with [[Get]] before the protocol method is consulted (22.1.3.13, 22.1.3.20).
const t=(s)=>{try{console.log(s, require('util').inspect(eval(s)))}catch(e){console.log(s, e.constructor.name+': '+e.message)}};
for (const s of [
"'abc'.replaceAll({[Symbol.match]:true, [Symbol.replace](s,r){return 'R'}},'y')",
"'abc'.replaceAll({[Symbol.match]:true, flags:'g', [Symbol.replace](s,r){return 'R'}},'y')",
"'abc'.replaceAll({[Symbol.match]:true, flags:'', [Symbol.replace](s,r){return 'R'}},'y')",
"'abc'.matchAll({[Symbol.match]:true, flags:'g', [Symbol.matchAll](s){return 'MA'}})",
"'abc'.matchAll({[Symbol.match]:true, flags:'', [Symbol.matchAll](s){return 'MA'}})",
"'abc'.matchAll({[Symbol.match]:true, [Symbol.matchAll](s){return 'MA'}})",
"(()=>{const r=/b/g; r[Symbol.match]=false; return 'abc'.replaceAll(r,'x')})()",
"(()=>{const r=/b/; r[Symbol.match]=false; return 'abc'.replaceAll(r,'x')})()",
"'abc'.replaceAll(/b/,'x')","'abc'.matchAll(/b/)",
"'a.b'.startsWith(/a/)","'a.b'.includes({[Symbol.match]:true})","'a.b'.endsWith((()=>{const r=/a/; r[Symbol.match]=false; return r})())",
"'x'.replaceAll({[Symbol.match]:true, get flags(){console.log('flags read'); return 'g'}}, 'y')",
]) t(s);
t("'aXbX'.replaceAll(/X/g, '-')"); t("[...'aXbX'.matchAll(/X/g)].map((m) => m.index)"); t("'abc'.matchAll({[Symbol.match]: true})");
