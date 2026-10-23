// The WebCrypto classes are globals, and `crypto` / `crypto.subtle` are their
// one instances: `instanceof`, prototype, brand, `constructor`, inherited
// (not owned) members, the brand-checked `subtle` getter, and node's
// refusals — `new` is ERR_ILLEGAL_CONSTRUCTOR, a plain call is an ES class
// call. `Crypto`, `CryptoKey` and `SubtleCrypto` were all undefined.
// (Only `digest` of `SubtleCrypto.prototype` is implemented, so only it is
// listed here.)
for (const n of ['Crypto', 'CryptoKey', 'SubtleCrypto']) console.log(n, typeof globalThis[n], n in globalThis);
console.log(crypto instanceof Crypto, crypto.subtle instanceof SubtleCrypto, Object.getPrototypeOf(crypto) === Crypto.prototype);
console.log(Object.prototype.toString.call(crypto), Object.prototype.toString.call(crypto.subtle));
for (const C of [Crypto, CryptoKey, SubtleCrypto]) { try { new C(); } catch (e) { console.log(C.name, e.name, e.code, e.message); } }
console.log(Object.getOwnPropertyNames(Crypto.prototype).sort().join());
console.log(Object.getOwnPropertyNames(CryptoKey.prototype).sort().join());
const c = require('crypto'); console.log(c.webcrypto === crypto, typeof c.webcrypto.CryptoKey, c.subtle === crypto.subtle);
console.log(Object.keys(crypto), Object.keys(crypto.subtle), crypto.hasOwnProperty('subtle'), crypto.hasOwnProperty('getRandomValues'));
console.log(crypto.getRandomValues === Crypto.prototype.getRandomValues, crypto.constructor === Crypto, crypto.subtle.constructor === SubtleCrypto);
console.log(crypto.subtle.digest === SubtleCrypto.prototype.digest, Object.getPrototypeOf(crypto.subtle) === SubtleCrypto.prototype);
try { Crypto(); } catch (e) { console.log(e.name, e.code, e.message); }
const d = Object.getOwnPropertyDescriptor(Crypto.prototype, 'subtle'); console.log(typeof d.get, d.enumerable);
try { d.get.call({}); } catch (e) { console.log(e.name, e.code, e.message); }
try { CryptoKey.prototype.type; } catch (e) { console.log(e.name, e.code, e.message); }
try{SubtleCrypto()}catch(e){console.log(e.message)}
try{new SubtleCrypto()}catch(e){console.log(e.message)}
const S=SubtleCrypto; try{S()}catch(e){console.log(e.message)}
console.log(Object.getOwnPropertyNames(SubtleCrypto.prototype).includes('digest'));
