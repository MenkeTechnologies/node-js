// Numeric literal lexing: exactly one `.` belongs to a literal, separators sit
// between digits, a literal may not run into an identifier, and the legacy
// octal/decimal forms. Each text is eval'd, so a rejection is a printed message.
const lits = [
  "5..toString()", "5.0.toFixed(1)", "1.e3", ".5e1", "5 .toString()", "0x10.toString()",
  "1e3.toString()", "0.5.toFixed()", "1..a", "1.5.toFixed(2)", "1.toString()", "1.5.5",
  "1_000", "0.0_1", "1e1_0", "0xAB_CD", "0b1_0", "0o7_7", "1_0n",
  "1_", "1__0", "0_1", "0_", "0x_1", "1_.5", "1._5", "1e_5", "0b1_", "0x1_",
  "0x", "0b", "0o", "0b12", "0o8", "1e", "1e+", ".e1", "3in []", "1a", "1$", "1_a",
  "0n", "10n", "0x1fn", "1.5n", "01n", "08n", "07n", "5.n",
  "010", "0777", "08", "09.5", "09e1", "08.5", "07.5", "07.toString()", "07e1", "07_", "08_1", "083_0",
  "00", "0e1", "0.e1", "0xFFFFFFFFFFFFFFFFF", "0b" + "1".repeat(64), "9007199254740993", "1e400",
];
for (const lit of lits) {
  let r;
  try { const v = eval(lit); r = typeof v === "bigint" ? v + "n" : String(v); } catch (e) { r = e.name + ": " + e.message; }
  console.log(JSON.stringify(lit), r);
}
