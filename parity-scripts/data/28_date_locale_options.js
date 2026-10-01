// Date.prototype.toLocale{,Date,Time}String with an options bag: the option
// set resolves to an ICU skeleton and then to en's pattern through the
// DateTimePatternGenerator distance search, so odd combinations get node's
// appended fields ("09, 9 (hour: 13)") and the date/time glue follows the
// month width (", " vs " at ").
const dates = [
  new Date(Date.UTC(2024, 1, 9, 13, 5, 9, 7)),
  new Date(Date.UTC(-50, 10, 23, 0, 0, 3, 450)),
  new Date(Date.UTC(2024, 6, 9, 12, 0, 0, 0)),
];
const methods = ['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString'];
function show(o) {
  const out = [];
  for (const d of dates) for (const m of methods) {
    try { out.push(d[m]('en-US', o)); } catch (e) { out.push(e.name + ': ' + e.message); }
  }
  console.log(JSON.stringify(o), '=>', out.join(' | '));
}

// Every combination of the component options, sampled.
const V = {
  weekday: [undefined, 'short', 'long', 'narrow'],
  era: [undefined, 'short', 'long'],
  year: [undefined, 'numeric', '2-digit'],
  month: [undefined, 'numeric', '2-digit', 'short', 'long', 'narrow'],
  day: [undefined, 'numeric', '2-digit'],
  dayPeriod: [undefined, 'short'],
  hour: [undefined, 'numeric', '2-digit'],
  minute: [undefined, 'numeric', '2-digit'],
  second: [undefined, 'numeric'],
  fractionalSecondDigits: [undefined, 2],
  timeZoneName: [undefined, 'short', 'longOffset'],
  hour12: [undefined, false],
  hourCycle: [undefined, 'h11', 'h24'],
};
const keys = Object.keys(V);
const total = keys.reduce((p, k) => p * V[k].length, 1);
// Decode combination number `k` as a mixed-radix number over the option lists.
for (let k = 0; k < total; k += 4999) {
  const o = {};
  let rest = k;
  for (const key of keys) {
    const v = V[key][rest % V[key].length];
    rest = Math.floor(rest / V[key].length);
    if (v !== undefined) o[key] = v;
  }
  show(o);
}

// Styles, alone and with an hour cycle.
for (const dateStyle of [undefined, 'full', 'long', 'medium', 'short'])
  for (const timeStyle of [undefined, 'full', 'long', 'medium', 'short'])
    for (const extra of [{}, { hour12: false }, { hourCycle: 'h11' }, { hourCycle: 'h24' }])
      show({ dateStyle, timeStyle, ...extra });

// Zones: UTC aliases, fixed offsets, named IANA zones across DST and LMT.
for (const timeZone of ['UTC', 'Etc/GMT-14', '+05:30', '-0800', 'America/New_York', 'Europe/London', 'Asia/Kolkata', 'Australia/Sydney', 'America/St_Johns'])
  for (const t of [0, Date.UTC(1900, 5, 1), Date.UTC(2024, 2, 10, 6, 59), Date.UTC(2024, 2, 10, 7, 0), Date.UTC(2050, 6, 1)])
    console.log(timeZone, new Date(t).toLocaleString('en-US', { timeZone, timeZoneName: 'short' }), '|', new Date(t).toLocaleString('en-US', { timeZone, timeZoneName: 'longOffset' }));

// Day periods across the day.
const periods = [];
for (let h = 0; h < 24; h += 3) periods.push(new Date(Date.UTC(2024, 0, 1, h)).toLocaleString('en-US', { hour: 'numeric', dayPeriod: 'long', timeZone: 'UTC' }));
console.log(periods.join(', '));

// Validation, in V8's order and wording.
for (const bad of [{ month: 'bogus' }, { weekday: 'numeric' }, { hourCycle: 'h99' }, { fractionalSecondDigits: 0 }, { fractionalSecondDigits: 2.9 }, { timeZone: 'Mars/Base' }, { timeZone: '+25:00' }, { dateStyle: 'full', month: 'long' }, { dateStyle: 'huge' }, null, 5])
  show(bad);
console.log(new Date(NaN).toLocaleString('en-US', { month: 'bogus' }));
