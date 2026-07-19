//! Differential parity harness (development tool): run the example corpus
//! through node-js and the reference `node`, diffing stdout. Needs `node` on
//! PATH, so CI never runs it. Frozen outputs live in
//! tests/data/parity_expected.txt for the no-`node` replay in tests/parity.rs.

use std::path::Path;
use std::process::Command;

fn main() {
    let dir = Path::new("examples");
    if !dir.exists() {
        eprintln!("parity: no examples/ directory");
        return;
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "js").unwrap_or(false))
        .collect();
    files.sort();

    // Our `node` binary is a sibling of this harness binary.
    let ours_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("node")))
        .unwrap_or_else(|| Path::new("node").to_path_buf());

    let mut pass = 0;
    let mut fail = 0;
    for f in &files {
        let ours = Command::new(&ours_bin)
            .arg(f)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let theirs = Command::new("node")
            .arg(f)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        match (ours, theirs) {
            (Some(a), Some(b)) if a == b => {
                pass += 1;
                println!("ok   {}", f.display());
            }
            (Some(a), Some(b)) => {
                fail += 1;
                println!("DIFF {}\n  ours: {a:?}\n  node: {b:?}", f.display());
            }
            (None, _) => {
                fail += 1;
                println!("ERR  {} (node-js failed to run)", f.display());
            }
            (_, None) => {
                println!("skip {} (no node)", f.display());
            }
        }
    }
    println!("\nparity: {pass} passed, {fail} failed");
}
