const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
for (const s of ['"\\x41"', '"\\u00"', '"\\u00zz"', '"\\ud83d\\ude00"', '"a\\', '"\\u004', '"\\q"', '"\\uD83D\\uDE00!"', '"\\u0041\\u00e9"'])
  console.log(JSON.stringify(s), P(() => JSON.stringify(JSON.parse(s))));
console.log(P(() => JSON.stringify(new Set([1]), ['1', 0], 1)));
console.log(P(() => JSON.stringify(/re/g, (k, v) => v, '\t')));
console.log(P(() => JSON.stringify(Object(Symbol('b')))), P(() => JSON.stringify([Object(Symbol('b'))])), P(() => JSON.stringify({ k: Object(Symbol('b')) })));
console.log(P(() => JSON.stringify(new Uint8Array([1, 2]), ['1', 0])));
console.log(P(() => JSON.stringify(new Uint8Array([1, 2]), ['1', 0], 2)));
console.log(P(() => JSON.stringify(Object.create({ a: 1 }), ['a'])), P(() => JSON.stringify(Object.defineProperty({}, 'x', { value: 1 }), ['x'])));
console.log(P(() => JSON.stringify(Object.assign(new Map(), { a: 1 }), null, 1)), P(() => JSON.stringify(Math, null, 2)));
