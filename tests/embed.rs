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
    // `is_err()` alone cannot tell this apart from a run that never reached the
    // `throw` — a parse failure, an unbound name, a panic in the host — all of
    // which are also `Err`, and the `out` assertion below would then be the only
    // thing failing. Pin the error the program actually raised.
    let err = result.expect_err("expected the throw to surface");
    assert_eq!(err, "Uncaught Error: boom", "wrong error surfaced: {err:?}");
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

// ── run_program: the uniform `(src, input) -> Result<stdout, error>` entry ──

#[test]
fn run_program_captures_stdout_only() {
    let out = nodejs::run_program(
        r#"process.stdout.write("raw "); console.log("log"); console.error("err");"#,
        "",
    );
    assert_eq!(out.as_deref(), Ok("raw log\n"));
}

/// `fs.readFileSync(0)` and a `readline` interface over `process.stdin` both
/// read the supplied input, not the test process's stdin.
#[test]
fn run_program_feeds_input_as_stdin() {
    let sync = nodejs::run_program(
        r#"console.log(require("fs").readFileSync(0, "utf8").toUpperCase())"#,
        "ab\ncd\n",
    );
    assert_eq!(sync.as_deref(), Ok("AB\nCD\n\n"));
    let lines = nodejs::run_program(
        r#"
        const rl = require("readline").createInterface({ input: process.stdin });
        rl.on("line", l => console.log(l.length));
        "#,
        "abc\nde\n",
    );
    assert_eq!(lines.as_deref(), Ok("3\n2\n"));
}

#[test]
fn run_program_errors_and_exit_status() {
    let thrown = nodejs::run_program("throw new Error('boom')", "").unwrap_err();
    assert!(thrown.contains("boom"), "{thrown}");
    assert!(nodejs::run_program("let = ;", "").is_err());
    assert!(nodejs::run_program("console.log('x'); process.exit(3)", "").is_err());
    assert!(nodejs::run_program("process.exitCode = 2", "").is_err());
    let early = nodejs::run_program("console.log('x'); process.exit(0); console.log('y')", "");
    assert_eq!(early.as_deref(), Ok("x\n"));
}

#[test]
fn run_program_concurrent_calls_do_not_mix_output() {
    let handles: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                let src = format!("for (let k = 0; k < 200; k++) console.log({i})");
                (i, nodejs::run_program(&src, "").unwrap())
            })
        })
        .collect();
    for h in handles {
        let (i, out) = h.join().unwrap();
        assert_eq!(out, format!("{i}\n").repeat(200));
    }
}
