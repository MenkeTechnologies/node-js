//! CI-safe parity replay: run node-js over every `examples/*.js` and assert its
//! stdout matches the FROZEN reference output captured from system `node`.
//!
//! Unlike the `parity` binary (which shells out to a live `node`), this test
//! needs no Node installed — it compares against tests/data/parity_expected.txt,
//! a snapshot regenerated only by running system `node` over the corpus. Editing
//! that file by hand to match a wrong node-js output would defeat its purpose;
//! it must always be regenerated from real `node`.
//!
//! Snapshot format: for each example (sorted by filename), a header line
//! `==== <basename> ====` followed by that program's exact stdout bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The compiled `node` binary under test (`CARGO_BIN_EXE_node` is set by cargo
/// for integration tests of a crate that declares the bin).
fn node_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_node"))
}

/// Sorted list of `examples/*.js`.
fn example_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("examples/ dir exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "js").unwrap_or(false))
        .collect();
    files.sort();
    files
}

/// Parse the frozen snapshot into (basename, stdout) records.
///
/// Two shapes the format cannot hold, both refused at record time by
/// `parity --bless` so the file can never contain one: a record with no
/// trailing newline (this splits on lines and re-adds one, so `"a"` and `"a\n"`
/// parse identically), and a body line that looks like a header (it would start
/// a new record here).
fn parse_expected(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("==== ") {
            let name = rest.trim_end_matches(" ====").to_string();
            out.push((name, String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    out
}

#[test]
fn examples_match_frozen_node_output() {
    let expected_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/parity_expected.txt");
    let expected_text = std::fs::read_to_string(&expected_path).unwrap_or_else(|e| {
        panic!(
            "missing frozen snapshot {}: {e}\n\
             regenerate with system `node` over examples/*.js",
            expected_path.display()
        )
    });
    let expected = parse_expected(&expected_text);
    let bin = node_bin();
    let files = example_files();

    assert_eq!(
        files.len(),
        expected.len(),
        "example count ({}) != frozen record count ({}); regenerate the snapshot",
        files.len(),
        expected.len()
    );

    let mut failures = Vec::new();
    for (f, (exp_name, exp_out)) in files.iter().zip(expected.iter()) {
        let base = f.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(&base, exp_name, "snapshot ordering mismatch");

        // The environment is pinned rather than inherited. The snapshot was
        // recorded from a reference `node` that is NOT locale- or TZ-invariant
        // (`toLocaleString`, the local-time `Date` getters), so a replay under a
        // different LANG/LC_ALL/TZ would be comparing against a transcript taken
        // under different conditions. node-js reads none of the three, so this
        // costs nothing and makes the test machine-independent by construction
        // rather than by luck.
        let out = Command::new(&bin)
            .arg(f)
            .env("TZ", "UTC")
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8")
            .output()
            .unwrap_or_else(|e| panic!("failed to run {}: {e}", bin.display()));
        // Compared as BYTES. `String::from_utf8_lossy` maps every invalid
        // sequence to `U+FFFD`, so two different invalid outputs compare equal —
        // an example writing raw bytes could diverge unreportably.
        if out.stdout != exp_out.as_bytes() {
            failures.push(format!(
                "DIFF {base}\n  frozen node: {exp_out:?}\n  node-js    : {:?}",
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        // The exit STATUS is part of what an example asserts and was not checked
        // at all. Every example in the corpus runs to completion under the
        // reference, so a non-zero status here is node-js failing where node did
        // not — including the silent form, `process.exitCode` left set, which
        // prints nothing and would otherwise pass.
        if out.status.code() != Some(0) {
            failures.push(format!(
                "EXIT {base}: node-js exited {:?}, the reference exits 0",
                out.status.code()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "node-js diverged from frozen node output:\n{}",
        failures.join("\n")
    );
}
