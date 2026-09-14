// Error-first callbacks. Every one of these APIs has a promise or sync form
// that already built a real Error, and a callback form that handed over a
// STRING — so the two halves of the same API disagreed about what a failure
// looks like, and `err instanceof Error`, `err.code` and `err.message` were all
// wrong on the callback side.
//
// Ordering is made deterministic by CHAINING: node runs children and zlib jobs
// on real async machinery, this runtime runs them synchronously and queues the
// callback, so the interleaving of two independent calls differs. Each step
// here starts the next from inside the previous callback, which pins the order
// to the call order in both.
const cp = require("child_process");
const z = require("zlib");

const line = (l, v) => console.log(l, JSON.stringify(v));

// `zlib` decode failures carry `code`/`errno`, and callers branch on them.
// A bare message left both `undefined`, and reported this runtime's own wording
// for a bad header (`corrupt deflate stream`) rather than zlib's.
const bad = Buffer.from([1, 2, 3]);
const sync = (f) => {
  try {
    f();
    return "no throw";
  } catch (e) {
    return [e instanceof Error, e.code, e.errno, e.message];
  }
};
line("gunzip-sync ", sync(() => z.gunzipSync(bad)));
line("inflate-sync", sync(() => z.inflateSync(bad)));
line("unzip-sync  ", sync(() => z.unzipSync(bad)));
// A stream whose HEADER is intact but which stops early is a different error:
// zlib calls that a BUF error, not a data error.
const truncated = z.gzipSync(Buffer.from("hello")).subarray(0, 6);
line("gunzip-trunc", sync(() => z.gunzipSync(truncated)));
// The good path still round-trips.
line("roundtrip   ", [z.gunzipSync(z.gzipSync(Buffer.from("hi"))).toString(), z.inflateSync(z.deflateSync(Buffer.from("yo"))).toString()]);

z.gunzip(bad, (e, out) => {
  // The async half must agree with the sync half it shares a decoder with.
  line("gunzip-cb   ", [e instanceof Error, e.code, e.errno, e.message, out]);

  z.gzip(Buffer.from("hi"), (e2, packed) =>
    z.gunzip(packed, (e3, out3) => {
      line("gzip-cb     ", [e2, e3, out3.toString()]);

      // `exec` failing: `err.code` is the EXIT STATUS (not a libuv string), and
      // the message carries the command's own stderr appended to it.
      cp.exec("echo out; echo err 1>&2; exit 7", (err, so, se) => {
        line("exec-fail   ", [err instanceof Error, err.code, err.killed, err.signal, err.cmd, err.message, so, se]);

        cp.exec("echo fine", (err2, so2, se2) => {
          line("exec-ok     ", [err2, so2, se2]);

          // A command the SHELL cannot find is still a successful spawn of the
          // shell — the status is the shell's 127, not a spawn error.
          cp.exec("no-such-command-xyz", (err3, so3, se3) => {
            line("exec-nocmd  ", [err3 instanceof Error, err3.code, so3, se3.includes("not found")]);

            // `execFile` runs the file DIRECTLY, so its `cmd` is the file and
            // its arguments joined rather than a shell command line.
            cp.execFile("sh", ["-c", "echo e 1>&2; exit 5"], (err4, so4, se4) => {
              line("execFile    ", [err4 instanceof Error, err4.code, err4.cmd, err4.killed, err4.signal, so4, se4]);

              cp.execFile("sh", ["-c", "exit 0"], (err5, so5) => {
                line("execFile-ok ", [err5, so5]);

                // A MISSING binary is a spawn failure, and it is reported
                // through the callback — `execFile` is async and never throws.
                // Throwing killed the script instead of running the handler.
                const child = cp.execFile("no-such-binary-xyz", [], (err6, so6, se6) => {
                  line("execFile-eno", [err6 instanceof Error, err6.code, err6.errno, err6.syscall, err6.path, err6.spawnargs, err6.message, so6, se6]);
                });
                line("returned    ", [typeof child, child !== null]);
              });
            });
          });
        });
      });
    }));
});
