// Four `os` entry points answered an empty or zero result: `os.cpus()` was
// `[]`, `totalmem`/`freemem` were 0, `uptime` was 0, and `networkInterfaces()`
// was `{}`. None of them failed — they reported a machine with no cores, no
// memory, no uptime and no network, which is the shape that makes downstream
// code misbehave quietly (`os.cpus().length || 1` sizing a pool at 1).
//
// The VALUES depend on the machine, so what is pinned here is the shape and the
// relations that hold everywhere. The parity harness runs both sides on the
// same host, so a relation like `freemem() <= totalmem()` is a real check.
const os = require("os");
const show = (label, f) => {
  try { console.log(label, JSON.stringify(f())); }
  catch (e) { console.log(label, e.constructor.name + ": " + e.message); }
};

// One entry per logical core, each with node's key set.
show("cpu-count    ", () => { const c = os.cpus(); return [Array.isArray(c), c.length > 0, c.length === os.availableParallelism()]; });
show("cpu-keys     ", () => { const c = os.cpus()[0]; return [Object.keys(c).sort(), Object.keys(c.times).sort()]; });
show("cpu-model    ", () => { const c = os.cpus()[0]; return [typeof c.model, c.model.length > 0, typeof c.speed, c.speed >= 0]; });
// Memory is real and ordered.
// `freemem() > 0`, not `>= 0`: a machine running this has some free memory, and
// zero is exactly what the stub answered. The exact figure cannot be pinned —
// it changes between the two process launches the harness compares — so the
// free-list-plus-speculative definition libuv uses is verified by hand against
// node rather than here.
show("memory       ", () => [os.totalmem() > 0, os.freemem() > 0, os.freemem() <= os.totalmem(), Number.isInteger(os.totalmem()), Number.isInteger(os.freemem())]);
// Uptime is a positive number of seconds — a machine running this cannot have
// been up for zero.
show("uptime       ", () => [typeof os.uptime(), os.uptime() > 0, Number.isFinite(os.uptime())]);
// Every interface has at least one address, in node's exact shape.
show("net-nonempty ", () => { const n = os.networkInterfaces(); return [typeof n, Object.keys(n).length > 0, Object.values(n).every((l) => Array.isArray(l) && l.length > 0)]; });
show("net-keys     ", () => { const a = Object.values(os.networkInterfaces()).flat()[0]; return Object.keys(a).sort(); });
show("net-families ", () => { const f = new Set(); for (const l of Object.values(os.networkInterfaces())) for (const a of l) f.add(a.family); return [...f].sort().every((x) => x === "IPv4" || x === "IPv6"); });
// The loopback entry is the one every machine has, with fixed values.
show("net-loopback ", () => { const n = os.networkInterfaces(); const lo = n.lo0 || n.lo; const v4 = lo.find((a) => a.family === "IPv4"); return [v4.address, v4.netmask, v4.internal, v4.cidr]; });
// A MAC is six hex pairs, and `cidr` is the address plus a prefix length.
show("net-mac      ", () => Object.values(os.networkInterfaces()).flat().every((a) => /^[0-9a-f]{2}(:[0-9a-f]{2}){5}$/.test(a.mac)));
show("net-cidr     ", () => Object.values(os.networkInterfaces()).flat().every((a) => a.cidr === `${a.address}/${a.cidr.split("/")[1]}` && Number.isInteger(+a.cidr.split("/")[1])));
show("net-internal ", () => { const n = os.networkInterfaces(); const lo = n.lo0 || n.lo; return [lo.every((a) => a.internal === true), Object.values(n).flat().every((a) => typeof a.internal === "boolean")]; });
// The neighbours that already worked, so a regression in them is visible too.
show("neighbours   ", () => [typeof os.hostname(), os.homedir().length > 0, os.loadavg().length === 3, typeof os.endianness(), os.EOL === "\n"]);
