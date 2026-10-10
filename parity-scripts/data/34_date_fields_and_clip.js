// Date: a setter with no argument is ToNumber(undefined); Date.UTC and
// `new Date` clip at ±8.64e15 (and yield +0, never -0); the readable string
// forms pad a negative year as -0001; Date() without `new` is a string.
const P = (f) => { try { const v = f(); return Object.is(v, -0) ? "-0" : String(v); } catch (e) { return e.name + ": " + e.message; } };
const times = [0, -1, 1e12, 8.64e15, -8.64e15, 951782400000, -62198755200000, -62167219200000, 253402300799999, -0.5, NaN];
const setters = ["setUTCMilliseconds", "setUTCSeconds", "setUTCMinutes", "setUTCHours", "setUTCDate", "setUTCMonth", "setUTCFullYear", "setTime"];
for (const t of times) {
  const d = new Date(t);
  console.log(String(t), P(() => d.toISOString()), P(() => d.toUTCString()), P(() => String(d)), P(() => d.toDateString()), P(() => d.getTime()));
}
for (const s of setters) {
  console.log(s, ["", "0", "1", "-1", "59", "NaN", "undefined", "'7'", "1.9", "-0.5, 3", "1e10"].map((a) => {
    const d = new Date(951782400000);
    return P(() => eval(`d.${s}(${a})`)) + "/" + P(() => d.getTime());
  }).join(" "));
}
for (const args of ["275760, 8, 13", "275760, 8, 13, 0, 0, 0, 1", "-271821, 3, 20", "-271821, 3, 19, 23, 59, 59, 999", "99", "100, 0", "2020, 1, 30", "NaN", "", "2020.9, 1.9, 1.9", "1970, 0, 1, 0, 0, 0, 0.9", "-0.5"]) {
  console.log(args, P(() => eval(`Date.UTC(${args})`)));
}
console.log(typeof Date(), typeof Date(0), Date.length, Date.name, new Date(-0.5).getTime() === 0, Object.is(new Date(-0.5).getTime(), 0));
