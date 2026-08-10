//! Differential parity harness (development tool): run the example corpus
//! through node-js and the reference `node`, comparing stdout BYTES and the
//! exit status. Needs `node` on PATH, so CI never runs it. Frozen outputs live
//! in tests/data/parity_expected.txt for the no-`node` replay in tests/parity.rs,
//! and `--bless` is the one supported way to regenerate them.
//!
//! Usage:
//!   parity            compare the corpus against the reference
//!   parity --bless    re-RECORD tests/data/parity_expected.txt from the
//!                     reference, then compare
//!
//! What this tool used to do, and why each of those is a defect a measuring
//! tool cannot have:
//!
//! * It always exited 0. A failing run printed `DIFF` lines and reported
//!   success to whatever invoked it, so it could never gate anything.
//! * It compared `String::from_utf8_lossy` of each side. Two DIFFERENT invalid
//!   byte sequences both render to `U+FFFD` and compare equal, so a divergence
//!   in non-UTF-8 output was unreportable — the exact hazard the fuzzer's own
//!   doc comment warns about, in the sibling tool.
//! * With no `node` installed every case printed `skip` and the summary read
//!   `0 passed, 0 failed`, exit 0 — a full-green-looking run that compared
//!   nothing at all. `run.sh` grew a gate against precisely this shape; this
//!   tool never did.
//! * It ignored the exit status entirely.
//! * It had no timeout, so one hanging example hung the harness forever.
//! * It inherited the developer's LANG/LC_ALL/TZ, which reference `node` is not
//!   invariant under.
//!
//! And the frozen snapshot it feeds had NO writer anywhere in the repo: it was
//! maintained by hand, which is the condition under which a recorded expectation
//! can describe output no version of the reference ever produced. `--bless`
//! records from the REFERENCE process only — node-js's output is never a source
//! for the file — so a blessed record is a transcript by construction.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Per-case wall-clock limit. An example that outruns it never reached a
/// verdict, which is a gate failure rather than a pass.
const TIMEOUT: Duration = Duration::from_secs(15);

struct RunOut {
    stdout: Vec<u8>,
    exit: i32,
    timed_out: bool,
    spawn_failed: bool,
}

/// Run `prog <file>` with a watchdog, capturing stdout as raw BYTES.
///
/// The locale and timezone are pinned rather than inherited: reference `node`
/// answers `toLocaleString` and the local-time `Date` getters differently under
/// a different LANG/LC_ALL/TZ, so an inherited environment makes a result
/// specific to the machine that produced it. node-js reads none of the three.
fn run(prog: &Path, file: &Path) -> RunOut {
    let mut child = match Command::new(prog)
        .arg(file)
        .env("TZ", "UTC")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => {
            return RunOut {
                stdout: Vec::new(),
                exit: -1,
                timed_out: false,
                spawn_failed: true,
            }
        }
    };
    let reader = child.stdout.take().map(|mut o| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = o.read_to_end(&mut b);
            b
        })
    });
    let deadline = Instant::now() + TIMEOUT;
    let mut timed_out = false;
    let exit;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                exit = s.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    exit = -1;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                exit = -1;
                break;
            }
        }
    }
    let stdout = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    RunOut {
        stdout,
        exit,
        timed_out,
        spawn_failed: false,
    }
}

/// Render bytes for a report. Invalid UTF-8 is shown lossily AND followed by a
/// hex line, because two different invalid sequences both render to `U+FFFD`
/// and would otherwise show a real divergence as identical text.
fn render(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if std::str::from_utf8(bytes).is_err() {
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        return format!("{text:?}\n       (hex) {}", hex.join(" "));
    }
    format!("{text:?}")
}

fn example_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|e| e == "js").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn main() -> std::process::ExitCode {
    let bless = std::env::args().any(|a| a == "--bless");
    let dir = Path::new("examples");
    if !dir.exists() {
        eprintln!("parity: no examples/ directory (run from the crate root)");
        return std::process::ExitCode::FAILURE;
    }
    let files = example_files(dir);
    if files.is_empty() {
        eprintln!("parity: GATE FAIL — examples/ holds no .js files, so this run compared nothing");
        return std::process::ExitCode::FAILURE;
    }

    // Our `node` binary is a sibling of this harness binary.
    let ours = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("node")))
        .unwrap_or_else(|| Path::new("node").to_path_buf());
    let oracle = PathBuf::from(std::env::var("NODE_JS_PARITY_NODE").unwrap_or("node".into()));

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut unusable: Vec<String> = Vec::new();
    let mut records: Vec<(String, Vec<u8>)> = Vec::new();

    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let theirs = run(&oracle, f);
        let mine = run(&ours, f);

        // A side that could not be run, or that never reached a verdict, did not
        // produce a comparison. Counting it as anything but a failure is how a
        // run that verified nothing reports green.
        if theirs.spawn_failed {
            unusable.push(format!(
                "{name}: reference '{}' not runnable",
                oracle.display()
            ));
            continue;
        }
        if mine.spawn_failed {
            unusable.push(format!("{name}: node-js '{}' not runnable", ours.display()));
            continue;
        }
        if theirs.timed_out || mine.timed_out {
            unusable.push(format!(
                "{name}: timed out (node={}, node-js={})",
                theirs.timed_out, mine.timed_out
            ));
            continue;
        }

        // Recorded from the REFERENCE only. node-js's output is never a source
        // for the frozen file — a snapshot taken from the implementation under
        // test would agree with it by construction and measure nothing.
        records.push((name.clone(), theirs.stdout.clone()));

        if theirs.stdout == mine.stdout && theirs.exit == mine.exit {
            pass += 1;
            println!("ok   {name}");
        } else {
            fail += 1;
            println!(
                "DIFF {name}\n  node   : exit={} {}\n  node-js: exit={} {}",
                theirs.exit,
                render(&theirs.stdout),
                mine.exit,
                render(&mine.stdout)
            );
        }
    }

    if bless && unusable.is_empty() {
        let path = Path::new("tests/data/parity_expected.txt");
        let mut buf: Vec<u8> = Vec::new();
        for (name, out) in &records {
            // The snapshot format cannot represent two shapes, and writing one
            // would produce a file whose reader can never reproduce it: a record
            // with NO trailing newline (the reader splits on lines and re-adds
            // one, so `"a"` and `"a\n"` are the same record), and a record whose
            // body contains a line that looks like a header (the reader would
            // split the record in two). Refuse rather than record something the
            // replay cannot match — a snapshot that disagrees with itself is the
            // worst outcome available here.
            if !out.is_empty() && !out.ends_with(b"\n") {
                eprintln!(
                    "parity: refusing to bless {name}: its stdout has no trailing newline, \
                     which the snapshot format cannot represent"
                );
                return std::process::ExitCode::FAILURE;
            }
            if String::from_utf8_lossy(out)
                .lines()
                .any(|l| l.starts_with("==== ") && l.ends_with(" ===="))
            {
                eprintln!(
                    "parity: refusing to bless {name}: its stdout contains a line the snapshot \
                     reader would parse as a record header"
                );
                return std::process::ExitCode::FAILURE;
            }
            buf.extend_from_slice(format!("==== {name} ====\n").as_bytes());
            buf.extend_from_slice(out);
        }
        match std::fs::write(path, &buf) {
            Ok(()) => println!("\nblessed {} record(s) → {}", records.len(), path.display()),
            Err(e) => {
                eprintln!("parity: cannot write {}: {e}", path.display());
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    println!(
        "\nparity: {pass} passed, {fail} failed, {} unusable",
        unusable.len()
    );
    for u in &unusable {
        println!("  UNUSABLE  {u}");
    }
    if !unusable.is_empty() {
        println!(
            "GATE FAIL: {} case(s) never produced a comparison — this run did not verify them.",
            unusable.len()
        );
    }
    if pass + fail == 0 {
        println!(
            "GATE FAIL: {} case(s) found but NONE was compared.",
            files.len()
        );
    }
    if fail == 0 && unusable.is_empty() && pass > 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
