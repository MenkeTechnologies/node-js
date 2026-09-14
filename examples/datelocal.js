// The LOCAL half of `Date`. Every local getter shared its arm with the `getUTC*`
// one, `getTimezoneOffset()` answered 0, and `toString` rendered the UTC wall
// clock — so under `TZ=America/Detroit`, `new Date(0).getMonth()` was 0 where
// node says 11.
//
// The parity harness pins `TZ=UTC` for both sides (deliberately — a transcript
// taken in one zone is not replayable in another), so what this record can pin
// is the UTC rendering plus the INVARIANTS that hold in every zone. The
// zone-specific verification is a probe run by hand against real node under
// three zones; see the commit.
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};
const d = new Date(0);

// Under any zone, the local clock is the UTC clock shifted by the offset.
show("offset-shape ", () => { const off = d.getTimezoneOffset(); return [typeof off, Number.isFinite(off), off % 15 === 0]; });
show("local-vs-utc ", () => { const shifted = new Date(d.getTime() - d.getTimezoneOffset() * 60000); return [d.getHours() === shifted.getUTCHours(), d.getMinutes() === shifted.getUTCMinutes(), d.getDate() === shifted.getUTCDate(), d.getMonth() === shifted.getUTCMonth(), d.getFullYear() === shifted.getUTCFullYear(), d.getDay() === shifted.getUTCDay()]; });
// A local setter round-trips through its own getter, whatever the zone.
show("roundtrip    ", () => { const x = new Date(0); x.setHours(x.getHours()); x.setDate(x.getDate()); x.setFullYear(x.getFullYear()); return x.getTime() === 0; });
show("set-reads    ", () => { const x = new Date(0); x.setHours(5); x.setMinutes(7); return [x.getHours(), x.getMinutes()]; });
show("set-utc-reads", () => { const x = new Date(0); x.setUTCHours(5); x.setUTCMinutes(7); return [x.getUTCHours(), x.getUTCMinutes()]; });
// The UTC half is unaffected by the zone, so these are fixed values.
show("utc-getters  ", () => [d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), d.getUTCHours(), d.getUTCDay()]);
show("iso          ", () => [d.toISOString(), d.toJSON(), new Date(d.toISOString()).getTime()]);
show("utc-string   ", () => d.toUTCString());
show("timestamp    ", () => [d.getTime(), d.valueOf(), +d]);
// `toString` carries the local clock, the numeric offset and a zone name. The
// name comes from ICU in node and from the C library here, so only its SHAPE is
// pinned — a zone-specific spelling would pin the machine (see BUGS.md).
show("to-string    ", () => { const s = d.toString(); const off = -d.getTimezoneOffset(); const sign = off < 0 ? "-" : "+"; const hh = String(Math.floor(Math.abs(off) / 60)).padStart(2, "0"); const mm = String(Math.abs(off) % 60).padStart(2, "0"); return [s.startsWith(d.toDateString()), s.includes(` GMT${sign}${hh}${mm} (`), s.endsWith(")"), d.toTimeString().startsWith(s.slice(d.toDateString().length + 1, d.toDateString().length + 9))]; });
show("date-string  ", () => [d.toDateString(), d.toDateString() === new Date(d.getTime()).toDateString()]);
show("locale-shape ", () => { const l = d.toLocaleString(); return [l === `${d.toLocaleDateString()}, ${d.toLocaleTimeString()}`, /^\d+\/\d+\/\d+$/.test(d.toLocaleDateString()), /^\d+:\d\d:\d\d [AP]M$/.test(d.toLocaleTimeString())]; });
// An Invalid Date answers NaN from every accessor and renders one string.
show("invalid      ", () => { const x = new Date(NaN); return [x.getHours(), x.getTimezoneOffset(), x.toString(), x.toDateString()]; });
show("invalid-set  ", () => { const x = new Date(NaN); x.setHours(5); return x.getTime(); });
show("invalid-year ", () => { const x = new Date(NaN); x.setFullYear(2000); return [x.getUTCFullYear(), Number.isNaN(x.getTime())]; });
// A DST transition is per-timestamp, so two instants in one zone can differ.
show("dst-pair     ", () => { const jan = new Date("2021-01-01T12:00:00Z"), jul = new Date("2021-07-01T12:00:00Z"); const a = jan.getTimezoneOffset(), b = jul.getTimezoneOffset(); return [Number.isFinite(a), Number.isFinite(b), a - b === 0 || Math.abs(a - b) === 60]; });
// Converting a local wall clock BACK to a timestamp needs the offset at the
// result, not at the guess. Every local midnight of a year is round-tripped
// two ways, which disagree only if the second lookup is missing; and a time
// inside a spring-forward GAP — one the wall clock never showed — is resolved
// the way V8 resolves it.
show("roundtrip-year", () => { let diffs = 0; for (let day = 0; day < 366; day++) { const base = Date.UTC(2021, 0, 1) + day * 86400000; const x = new Date(base); x.setHours(2, 30, 0, 0); const y = new Date(base); y.setHours(2); y.setMinutes(30); y.setSeconds(0); y.setMilliseconds(0); if (x.getTime() !== y.getTime()) diffs++; } return diffs; });
show("dst-gap      ", () => { const t = new Date("2021-03-14T06:30:00Z"); t.setHours(2, 30); const u = new Date("2021-11-07T05:30:00Z"); u.setHours(1, 30); return [t.getHours(), t.getMinutes(), u.getHours(), u.getMinutes()]; });
// …and `toString`'s clock is the LOCAL one, which a UTC rendering fails in any
// zone with a non-zero offset and matches trivially in UTC.
show("string-clock ", () => { const s = d.toString(); const hh = String(d.getHours()).padStart(2, "0"); const mm = String(d.getMinutes()).padStart(2, "0"); return [s.includes(` ${hh}:${mm}:`), d.toTimeString().startsWith(`${hh}:${mm}:`)]; });
