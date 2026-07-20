// Tagged templates and String.raw.
function highlight(strings, ...values) {
  return strings.reduce((acc, str, i) => {
    const val = i < values.length ? `<${values[i]}>` : "";
    return acc + str + val;
  }, "");
}

const name = "Ada";
const lang = "JS";
console.log(highlight`Hello ${name}, welcome to ${lang}!`);

// String.raw preserves escapes.
const path = String.raw`C:\Users\name\temp`;
console.log(path);
console.log(String.raw`line1\nline2\t\tend`);

// Tag that uppercases interpolations.
function upper(strings, ...values) {
  let out = "";
  strings.forEach((s, i) => {
    out += s;
    if (i < values.length) out += String(values[i]).toUpperCase();
  });
  return out;
}
console.log(upper`the ${"quick"} brown ${"fox"}`);

// Access raw vs cooked.
function inspect(strings) {
  return `cooked=[${strings.join("|")}] raw=[${strings.raw.join("|")}]`;
}
console.log(inspect`a\nb`);

// Build a simple query-like string.
function sql(strings, ...vals) {
  return strings.reduce(
    (acc, s, i) => acc + s + (i < vals.length ? `'${vals[i]}'` : ""),
    "",
  );
}
console.log(sql`SELECT * FROM users WHERE name = ${name} AND lang = ${lang}`);
