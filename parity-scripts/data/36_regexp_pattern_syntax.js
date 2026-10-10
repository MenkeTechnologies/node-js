// Pattern grammar errors and the wording V8 gives each, with and without `u`.
const pats = ["(", ")", "(a", "a)", "[", "[a", "[b-a]", "[\\d-x]", "a{2,1}", "a{1", "{", "}", "]", "{1}", "*", "a**", "a|*",
  "^*", "\\b+", "$+", "(?=a)+", "(?<=a)+", "(?:a)+", "(?!a)*", "\\", "\\c", "\\x4", "\\u12", "\\u{110000}", "\\u{61}", "\\-", "\\_",
  "\\1", "\\01", "\\k<a>", "(?<a>.)\\k<a>", "(?<a>.)(?<a>.)", "(?<a>.)|(?<a>.)", "(?<>a)", "(?<1a>a)", "(?<a",
  "\\p{L}", "\\p{Foo}", "[\\p{Foo}]", "\\p{Script=Greek}", "(?i)", "(?i:a)", "(?P<n>a)", "(?#c)", "(?>a)", "[\\b]", "[\\B]",
  "\\0", "\\00", "[\\0]", "a{,5}", "x{2}{3}", "\\/", "[/]", "\\ud83d\\ude00", "[\\u{1F600}-\\u{1F64F}]", "[a-\\d]", "(?<=a)b", "(?<!a)b"];
for (const flags of ["", "u", "v", "i"]) {
  for (const p of pats) {
    let r;
    try { r = String(new RegExp(p, flags)); } catch (e) { r = e.name + ": " + e.message; }
    console.log(JSON.stringify(flags), JSON.stringify(p), r);
  }
}
for (const f of ["gg", "uv", "x", "dgimsvy", "dgimsuy", "yy", "G"]) {
  try { console.log(new RegExp("a", f).flags); } catch (e) { console.log(e.name + ": " + e.message); }
}
