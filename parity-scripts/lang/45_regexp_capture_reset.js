// RepeatMatcher step 4 clears the captures inside a group at the start of every iteration.
console.log(/(z)((a+)?(b+)?(c))*/.exec('zaacbbbcac'));
console.log(/(?:(a)|b)*/.exec('ab'));
console.log(/(?:(a)|(b))+/.exec('ab'));
console.log(/(a)|b/.exec('b'));
console.log(/(?:(a)|b){2}/.exec('ab'), /(?:(a)|b){2}/.exec('ba'));
console.log('aab'.replace(/(?:(a)|(b))+/, '[$1|$2]'));
console.log(/(?<x>a)|(?<y>b)/.exec('b').groups);
console.log(/(?:(?:(a)|b)c)*/.exec('acbc'));
console.log(/(a*)*/.exec('b'), /(a*)+/.exec('b'), /(a*)?/.exec('b'), /(?:(a*))?/.exec('b'));
console.log(/(?:(a)|(b)){0,2}x/.exec('abx'));
console.log('abc'.split(/(?:(a)|(b))+/));
console.log([...'ab'.matchAll(/(?:(a)|(b))+/g)].map((m) => m.slice()));
console.log(/(?:a(b)?)+/.exec('aba'), /(?:(?=(a))a|b)+/.exec('ab'));
console.log(/(a*)*b/.exec('aab'), /(a|)*/.exec('ab'), /(a*){2,}/.exec('aa'));
