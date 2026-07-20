// Regex replace with $1, $<name>, function replacer.
console.log("2026-07-20".replace(/(\d{4})-(\d{2})-(\d{2})/, "$3/$2/$1"));
console.log("John Smith".replace(/(\w+)\s+(\w+)/, "$2, $1"));
console.log("hello".replace(/l/g, "[$&]"));       // $& = whole match

// named groups + $<name>
console.log("2026-07-20".replace(/(?<y>\d{4})-(?<m>\d{2})-(?<d>\d{2})/, "$<d>.$<m>.$<y>"));

// function replacer
console.log("a1b2c3".replace(/\d/g, (d) => String(Number(d) * 2)));
console.log("hello world".replace(/\w+/g, (w) => w.toUpperCase()));
console.log("abc".replace(/./g, (c, i) => `${c}${i}`));

// named group via exec .groups
const m = /(?<area>\d{3})-(?<num>\d{4})/.exec("call 555-1234 now");
console.log(m.groups.area, m.groups.num);

console.log("  a  b   c ".replace(/\s+/g, " ").trim());
console.log("price: 42 and 100".replace(/\d+/g, (n) => `$${n}`));
console.log("snake_case_word".replace(/_(\w)/g, (_, c) => c.toUpperCase()));
console.log("aaa".replace(/a/, "b"));            // first only
