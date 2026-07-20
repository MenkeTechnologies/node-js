//! End-to-end inline Rust FFI: a `rust { ... }` block is desugared, compiled to
//! a cdylib via `rustc`, dlopened, and its exports called from JavaScript.
//! Requires `rustc` on PATH (always present in a Rust CI); skips cleanly
//! otherwise so a toolchain-less environment never reports a false failure.
//!
//! Drives the built `node` binary as a subprocess (`CARGO_BIN_EXE_node`):
//! `console.log` writes straight to the process stdout, and running out of
//! process also isolates the FFI dlopen/registry from the test harness.

use std::io::Write;
use std::process::Command;

fn rustc_available() -> bool {
    Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Write `src` to a temp `.js` file and run it through the built `node` binary,
/// returning `(stdout, stderr, success)`.
fn run_js(src: &str) -> (String, String, bool) {
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(src.as_bytes()).expect("write source");
    let path = f.path().to_owned();
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(&path)
        .output()
        .expect("spawn node binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn rust_block_exports_are_callable_across_all_v1_signatures() {
    if !rustc_available() {
        eprintln!("skipping FFI test: rustc not on PATH");
        return;
    }
    // Distinct names so this test's registry entries never collide with another
    // test's. Exercises int-arity, float-arity, and string->int marshalling
    // (the string arg rides as a JS heap handle and is marshalled to a native
    // fusevm string before the call).
    let src = r#"
rust {
    pub extern "C" fn ffi_addi(a: i64, b: i64) -> i64 { a + b }
    pub extern "C" fn ffi_mulf(x: f64, y: f64, z: f64) -> f64 { x * y * z }
    pub extern "C" fn ffi_slen(s: *const c_char) -> i64 {
        unsafe { CStr::from_ptr(s).to_bytes().len() as i64 }
    }
}
console.log(ffi_addi(21, 21))
console.log(ffi_mulf(1.5, 2.0, 3.0))
console.log(ffi_slen("hello world"))
"#;
    let (stdout, stderr, ok) = run_js(src);
    assert!(ok, "FFI program failed: stderr={stderr}");
    assert_eq!(stdout, "42\n9\n11\n", "stderr={stderr}");
}

#[test]
fn rust_block_with_no_exports_errors() {
    if !rustc_available() {
        return;
    }
    // A block with no `pub extern "C" fn` is a hard error — v1 requires at least
    // one exported function.
    let src = "rust { fn helper() -> i64 { 1 } }\nconsole.log(1)\n";
    let (_stdout, stderr, ok) = run_js(src);
    assert!(!ok, "empty-export block must error");
    assert!(stderr.contains("rust FFI"), "unexpected error: {stderr}");
}
