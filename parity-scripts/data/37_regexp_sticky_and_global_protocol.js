// `y` and `g` through the String methods: where the search begins, what
// lastIndex becomes, a non-global/non-sticky `match` leaving lastIndex alone,
// matchAll reading the ORIGINAL's lastIndex, and `$<name>` expansion.
const subjects = ["aaba", "baa", "aab", "abab", "", "xaay"];
const pats = ["a", "a+", "b", "(?:)", "a|b", "(?<x>a)", "\\w", "a*?"];
const flagsets = ["", "g", "y", "gy", "d"];
const repls = ["X", "[$&]", "$<x>", "$<x", "$1$2", "$$", "$`|$'"];
for (const s of subjects) for (const p of pats) for (const fl of flagsets) for (const li of [0, 1, 3]) {
  const mk = () => { const r = new RegExp(p, fl); r.lastIndex = li; return r; };
  const out = [];
  let r = mk(); out.push(JSON.stringify(s.match(r)), r.lastIndex);
  r = mk(); out.push(s.search(r), r.lastIndex);
  r = mk(); out.push(JSON.stringify(r.exec(s)), r.lastIndex);
  r = mk(); out.push(s.replace(r, "X"), r.lastIndex);
  if (fl.includes("g")) { r = mk(); out.push(JSON.stringify([...s.matchAll(r)].map((m) => m.index)), r.lastIndex); }
  console.log(JSON.stringify(s), p, JSON.stringify(fl), li, out.join(" "));
}
for (const rp of repls) {
  console.log(JSON.stringify(rp), JSON.stringify("abc".replace(/(?<x>b)/, rp)), JSON.stringify("abc".replace(/b/, rp)), JSON.stringify("abcb".replace(/(b)/g, rp)));
}
