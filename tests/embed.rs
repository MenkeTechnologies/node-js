//! Tests for the embedder entry point (`eval_str_captured`), which an in-process
//! host uses instead of `eval_str`: globals are seeded after the host reset, and
//! program output is captured rather than written to the process streams.
//!
//! These run in-process (not through the `node` binary like the parity suites),
//! because that is precisely what is under test.

/// A seeded global is visible to the program. `eval_str` cannot do this — it
/// resets the host, so anything installed beforehand is gone by the time the
/// program runs.
#[test]
fn seeded_globals_survive_the_host_reset() {
    let (result, out) =
        nodejs::eval_str_captured("console.log(stdin.toUpperCase())", &[("stdin", "hello")]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(out, "HELLO\n");
}

/// Everything the program writes is captured: `console.log`, `console.error`
/// and the raw `process.stdout.write` path all funnel through the same host
/// sink, so none of them can reach a terminal the embedder owns.
#[test]
fn every_write_path_is_captured() {
    let (result, out) = nodejs::eval_str_captured(
        r#"
        process.stdout.write("raw ");
        console.log("log");
        console.error("err");
        "#,
        &[],
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(out, "raw log\nerr\n");
}

/// A program that prints and then throws produced both: the error and the
/// output it managed to write are returned side by side.
#[test]
fn output_before_a_throw_is_kept() {
    let (result, out) =
        nodejs::eval_str_captured("console.log('before'); throw new Error('boom');", &[]);
    assert!(result.is_err(), "expected the throw to surface");
    assert_eq!(out, "before\n");
}

/// Capture is per-run: a second run does not see the first run's output.
#[test]
fn capture_resets_between_runs() {
    let (_, first) = nodejs::eval_str_captured("console.log('one')", &[]);
    let (_, second) = nodejs::eval_str_captured("console.log('two')", &[]);
    assert_eq!(first, "one\n");
    assert_eq!(second, "two\n");
}

/// A seeded global is a real JS string, not merely something that stringifies:
/// it answers `typeof` and its own methods. Strings live on the host heap, so
/// this is the property a `Value`-shaped API would silently get wrong.
#[test]
fn seeded_globals_are_real_strings() {
    let (result, out) = nodejs::eval_str_captured("console.log(typeof stdin)", &[("stdin", "hi")]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(out, "string\n");
}
