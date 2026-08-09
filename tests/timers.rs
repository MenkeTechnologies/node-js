//! Timer liveness and `ref`/`unref` handle-counting parity.
//!
//! Every expected value here was captured from system `node v26.7.0`.
//!
//! The headline case is a **liveness** assertion, and liveness cannot be checked
//! by running a program to completion: real `node` never completes it. A
//! `setInterval` that is never cleared runs until the process is killed, so the
//! pass condition is that the child is *still running* when we come back for it
//! — the absence of a hang is the failure. A `setInterval` that quietly exited
//! would leave a poller or heartbeat silently doing nothing rather than failing
//! loudly, which is exactly the bug these tests exist to catch.
//!
//! "Still running" alone is too weak a signal, though: a deadlocked or spinning
//! process is also "still running". Each liveness test therefore *also* requires
//! the interval to have accumulated real ticks in a side file, so a process that
//! survives without doing its work still fails. The negative control
//! (`a_script_with_no_pending_work_still_exits`) guards the opposite error — a
//! loop that never exits would make every liveness test pass vacuously.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Write `src` to a `.js` file under `dir` and return its path.
fn write_js(dir: &Path, name: &str, src: &str) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.js"));
    let mut f = std::fs::File::create(&path).expect("create script");
    f.write_all(src.as_bytes()).expect("write script");
    path
}

/// Run `src` to completion, returning trimmed stdout. Panics on a non-zero exit
/// so a thrown error surfaces in the failure rather than as an empty string —
/// two empty outputs must never be mistaken for agreement.
fn run(src: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_js(dir.path(), "prog", src);
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(&path)
        .output()
        .expect("spawn node binary");
    if !out.status.success() {
        panic!(
            "program failed ({}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// How long to wait for a child to get as far as its first tick (or to exit).
///
/// This absorbs process start-up only, so it can be generous at no cost: the
/// wait ends the moment either event is observed. It needs to be generous. The
/// binary under test is an unoptimized debug build, and when the whole file's
/// tests run in parallel a dozen copies of it start at once against a cold page
/// cache — a trivial script measured 4.1s from spawn to exit under exactly that
/// load, versus ~70ms run on its own. An earlier version of this probe used a
/// flat 900ms window and reported three false failures that were purely
/// start-up latency, so the observation window below deliberately does not begin
/// until the child has proven it is up.
const STARTUP_GRACE: Duration = Duration::from_secs(60);

/// Outcome of a liveness probe.
struct Liveness {
    /// Whether the child was still running at the end of the observation window.
    still_running: bool,
    /// Ticks recorded by the time the child was up (or had exited).
    ticks_at_ready: usize,
    /// Ticks recorded when observation finished.
    ticks_final: usize,
    stderr: String,
}

/// Count non-empty lines in the tick log, treating a missing file as zero ticks.
fn count_ticks(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Spawn `src` and observe whether it keeps running.
///
/// `src` is handed the path of a tick log via `process.argv[2]`; every line it
/// appends counts as one tick.
///
/// Two phases, so that start-up cost is never mistaken for a liveness verdict:
///
/// 1. **Settle** — wait (up to [`STARTUP_GRACE`]) until the child either exits or
///    records its first tick. A child that exits here is simply not long-lived,
///    which is the correct answer for the `unref` cases.
/// 2. **Observe** — only once the child has proven it is up, watch it for
///    `observe`. Surviving that window *and* gaining ticks during it is what
///    distinguishes a live event loop from a wedged one.
///
/// The child is always killed and reaped, so a failing test cannot leak a
/// process that runs forever.
fn probe_liveness(src: &str, observe: Duration) -> Liveness {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_js(dir.path(), "live", src);
    let ticks_path = dir.path().join("ticks.log");

    let mut child = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(&path)
        .arg(&ticks_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn node binary");

    // Phase 1: absorb start-up. Ends at the first tick or at exit.
    let settle_deadline = Instant::now() + STARTUP_GRACE;
    let mut exited = false;
    while Instant::now() < settle_deadline {
        if child.try_wait().expect("try_wait").is_some() {
            exited = true;
            break;
        }
        if count_ticks(&ticks_path) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let ticks_at_ready = count_ticks(&ticks_path);

    // Phase 2: the child is up — now its survival actually means something.
    if !exited {
        let observe_deadline = Instant::now() + observe;
        while Instant::now() < observe_deadline {
            if child.try_wait().expect("try_wait").is_some() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    let still_running = !exited;
    if still_running {
        let _ = child.kill();
    }
    let out = child.wait_with_output().expect("reap child");

    Liveness {
        still_running,
        ticks_at_ready,
        ticks_final: count_ticks(&ticks_path),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

// ── liveness: a referenced timer holds the process open ──────────────────────

/// The target bug: `setInterval` must keep the process alive forever, the way
/// real `node` does (`timeout 3 node -e 'setInterval(function(){},1000)'` exits
/// 124, killed, having never finished on its own).
#[test]
fn set_interval_keeps_the_process_alive() {
    // A 50ms period so several real ticks land inside the observation window;
    // the script never clears the interval, so nothing should ever end it.
    let src = r#"
        const fs = require('fs');
        const log = process.argv[2];
        setInterval(function () { fs.appendFileSync(log, 'tick\n'); }, 50);
    "#;
    let live = probe_liveness(src, Duration::from_millis(600));
    assert!(
        live.still_running,
        "setInterval must keep the event loop alive, but the process exited on \
         its own after {} tick(s). stderr:\n{}",
        live.ticks_final, live.stderr
    );
    // Surviving is not enough — it has to still be doing the work. A wedged or
    // deadlocked loop is also "still running", and would stop gaining ticks.
    let gained = live.ticks_final - live.ticks_at_ready;
    assert!(
        gained >= 3,
        "a live 50ms interval should keep ticking: gained only {gained} tick(s) \
         during a 600ms observation window ({} → {}), so the process survived \
         without running its callback. stderr:\n{}",
        live.ticks_at_ready,
        live.ticks_final,
        live.stderr
    );
}

/// An `unref`'d interval must NOT hold the loop open, so this one exits — the
/// mirror image of the test above, proving liveness tracks the handle bit rather
/// than merely "an interval exists".
#[test]
fn an_unrefd_interval_does_not_hold_the_loop_open() {
    let src = r#"
        const fs = require('fs');
        const log = process.argv[2];
        const t = setInterval(function () { fs.appendFileSync(log, 'tick\n'); }, 50);
        t.unref();
    "#;
    let live = probe_liveness(src, Duration::from_millis(600));
    assert!(
        !live.still_running,
        "an unref'd interval must not keep the process alive, but it was still \
         running ({} ticks)",
        live.ticks_final
    );
    assert_eq!(
        live.ticks_final, 0,
        "an unref'd interval with nothing else pending should never fire"
    );
}

/// Negative control. Without this, a loop that simply never exits would make
/// every liveness assertion above pass for the wrong reason.
#[test]
fn a_script_with_no_pending_work_still_exits() {
    let src = r#"
        const fs = require('fs');
        fs.appendFileSync(process.argv[2], 'tick\n');
        setTimeout(function () { fs.appendFileSync(process.argv[2], 'tick\n'); }, 10);
    "#;
    let live = probe_liveness(src, Duration::from_millis(600));
    assert!(
        !live.still_running,
        "a script whose only timer is a fired setTimeout must exit; it did not, \
         so the loop is hanging unconditionally and the liveness tests above \
         prove nothing"
    );
    assert_eq!(
        live.ticks_final, 2,
        "both the sync write and the timeout should run"
    );
}

// ── setInterval repeats ──────────────────────────────────────────────────────

/// `setInterval` fires repeatedly until cleared. It previously fired exactly
/// once, which is why the process could exit at all.
#[test]
fn set_interval_repeats_until_cleared() {
    let src = r#"
        let n = 0;
        const t = setInterval(function () {
            n++;
            console.log('tick', n);
            if (n === 3) { clearInterval(t); console.log('cleared'); }
        }, 5);
    "#;
    assert_eq!(run(src), "tick 1\ntick 2\ntick 3\ncleared");
}

/// Clearing from *inside* the callback must stop the interval. The loop re-arms
/// a repeating timer before invoking it, precisely so the `clearInterval` here
/// has a live entry to cancel; re-arming afterwards would resurrect it.
#[test]
fn clear_interval_from_inside_the_callback_stops_it() {
    let src = r#"
        let n = 0;
        const t = setInterval(function () { n++; console.log('tick', n); if (n === 2) clearInterval(t); }, 5);
    "#;
    assert_eq!(run(src), "tick 1\ntick 2");
}

/// A repeating timer runs on the real clock, not the virtual one: five 60ms
/// ticks cannot complete in under 240ms. A virtual-clock interval would report a
/// near-zero elapsed time (and spin a core doing it).
#[test]
fn interval_ticks_on_the_real_clock() {
    let src = r#"
        const t0 = Date.now();
        let n = 0;
        const t = setInterval(function () {
            n++;
            if (n === 5) { clearInterval(t); console.log('elapsed_at_least_240ms:', (Date.now() - t0) >= 240); }
        }, 60);
    "#;
    assert_eq!(run(src), "elapsed_at_least_240ms: true");
}

// ── the Timeout / Immediate handle objects ───────────────────────────────────

/// `setTimeout`/`setInterval` return a `Timeout`; `setImmediate` returns an
/// `Immediate`. Both carry the handle methods — previously a bare number was
/// returned and `t.unref` was `undefined`.
#[test]
fn timers_return_handle_objects() {
    let src = r#"
        const t = setTimeout(function () {}, 10);
        console.log(typeof t, t.constructor.name);
        console.log(typeof t.ref, typeof t.unref, typeof t.hasRef, typeof t.refresh);
        clearTimeout(t);
        const i = setInterval(function () {}, 10);
        console.log(i.constructor.name);
        clearInterval(i);
        const im = setImmediate(function () {});
        console.log(im.constructor.name, typeof im.hasRef);
        clearImmediate(im);
    "#;
    assert_eq!(
        run(src),
        "object Timeout\nfunction function function function\nTimeout\nImmediate function"
    );
}

/// `hasRef()` tracks the handle bit, and `ref`/`unref`/`refresh` return the
/// handle so they chain. A cleared timer reports `hasRef() === false`.
#[test]
fn has_ref_tracks_ref_and_unref() {
    let src = r#"
        const t = setTimeout(function () {}, 50);
        console.log(t.hasRef());
        t.unref(); console.log(t.hasRef());
        t.ref();   console.log(t.hasRef());
        console.log(t.refresh() === t, t.ref() === t, t.unref() === t);
        t.ref();
        clearTimeout(t);
        console.log('after clear:', t.hasRef());
    "#;
    assert_eq!(
        run(src),
        "true\nfalse\ntrue\ntrue true true\nafter clear: false"
    );
}

/// A handle coerces to its integer timer id, which is what lets code that stored
/// the old numeric return value keep working. `clearTimeout` accepts either the
/// handle or that bare id.
#[test]
fn a_handle_coerces_to_its_timer_id_and_clears_by_either() {
    let src = r#"
        const t = setTimeout(function () { console.log('BUG: cleared timer fired'); }, 10);
        const id = Number(t);
        console.log(typeof id, id > 0, String(t) === String(id));
        clearTimeout(id);            // cleared by the bare id, not the handle
        const u = setTimeout(function () { console.log('BUG: cleared timer fired'); }, 10);
        clearTimeout(u);             // cleared by the handle itself
        setTimeout(function () { console.log('done'); }, 20);
    "#;
    assert_eq!(run(src), "number true true\ndone");
}

// ── unref semantics ──────────────────────────────────────────────────────────

/// An unref'd timer still fires while the loop is alive for another reason, but
/// is dropped when the loop would otherwise exit. Both halves are one test
/// because they are the same rule seen from either side.
#[test]
fn unrefd_timers_fire_only_while_something_else_holds_the_loop() {
    // The unref'd timer is due BEFORE the ref'd one, so the loop is still alive
    // when it comes due: it fires.
    let early = r#"
        const a = setTimeout(function () { console.log('unref-10'); }, 10);
        a.unref();
        setTimeout(function () { console.log('ref-20'); }, 20);
    "#;
    assert_eq!(run(early), "unref-10\nref-20");

    // The unref'd timer is due AFTER the ref'd one; once the ref'd timer has run
    // nothing holds the loop, so the process exits and it never fires.
    let late = r#"
        const a = setTimeout(function () { console.log('BUG: unref-50 fired'); }, 50);
        a.unref();
        setTimeout(function () { console.log('ref-10'); }, 10);
    "#;
    assert_eq!(run(late), "ref-10");
}

// ── ordering is unchanged by the liveness rework ─────────────────────────────

/// `process.nextTick` drains before promise microtasks, which drain before any
/// timer callback. Pinned here because a change to loop liveness could perturb
/// the order in which the queues are serviced.
///
/// Deliberately excludes `setTimeout(0)` vs `setImmediate` at the top level:
/// that race is genuinely nondeterministic in real `node` (measured across 12
/// runs of v26.7.0: 11 `immediate`-first, 1 `timeout`-first), so asserting
/// either order would pin behavior real Node does not guarantee.
#[test]
fn nexttick_drains_before_promises_which_drain_before_timers() {
    let src = r#"
        console.log('sync');
        setTimeout(function () { console.log('timeout'); }, 0);
        Promise.resolve().then(function () { console.log('promise'); });
        process.nextTick(function () { console.log('nextTick'); });
        console.log('sync-end');
    "#;
    assert_eq!(run(src), "sync\nsync-end\nnextTick\npromise\ntimeout");
}

/// Inside an I/O callback the `setImmediate`-before-`setTimeout` order *is*
/// deterministic in Node, so it is safe to pin.
#[test]
fn immediate_precedes_timeout_inside_an_io_callback() {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data.txt");
    std::fs::write(&data, b"x").expect("write data");
    let src = format!(
        r#"
        require('fs').readFile({:?}, function () {{
            setTimeout(function () {{ console.log('timeout'); }}, 0);
            setImmediate(function () {{ console.log('immediate'); }});
        }});
        "#,
        data.to_str().expect("utf8 path")
    );
    assert_eq!(run(&src), "immediate\ntimeout");
}
