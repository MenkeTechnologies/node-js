// \w \d \s \b are ASCII-only (\s is the spec's WhiteSpace + LineTerminator
// list, not Unicode's), `.` stops at all four LineTerminators, and `^`/`$`
// under `m` sit beside \r, U+2028 and U+2029 as well as \n.
const subjects = ["café ٣x", "a\u0085b﻿c", "a\rb\nc", "a b c", "ab cd_ef-gh", "x1y22z", " 　a", "a\r\nb"];
const res = [/\w+/g, /\W+/g, /\d+/g, /\D/g, /\s+/g, /\S+/g, /\b/g, /\B/g, /./g, /./gs, /^./gm, /.$/gm, /^/gm, /$/gm,
  /[\w-]+/g, /[^\d]+/g, /[\s\S]/g, /[\d-x]/g, /[\w\s]+/g, /[^\W\d]+/g, /\bc/g, /a$/m, /^b/m, /(?:)/g, /x?/g, /[\b]/, /\u{61}/, /\u{61}/u];
for (const s of subjects) {
  for (const re of res) {
    const m = s.match(re);
    console.log(JSON.stringify(s), String(re), JSON.stringify(m), JSON.stringify(s.replace(re, "[$&]")));
  }
}
