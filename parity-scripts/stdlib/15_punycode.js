// punycode: toASCII / toUnicode / encode / decode on fixed inputs.
// NOTE: node emits a deprecation warning to STDERR; only stdout is asserted.
const punycode = require("punycode");

console.log(punycode.toASCII("münchen.de"));
console.log(punycode.toASCII("例え.テスト"));
console.log(punycode.toUnicode("xn--mnchen-3ya.de"));
console.log(punycode.encode("münchen"));
console.log(punycode.decode("mnchen-3ya"));
console.log(punycode.toASCII("mañana.com"));
console.log(punycode.encode("abc"));
console.log(punycode.decode("maana-pta"));
