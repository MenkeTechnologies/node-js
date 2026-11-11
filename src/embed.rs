//! Uniform embedding entry point: run a whole program against a string stdin
//! and get its stdout back.

use std::cell::Cell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

thread_local! {
    /// Set on the thread [`run_program`] spawns: `process.exit` unwinds to it
    /// instead of ending the process.
    static EMBEDDED: Cell<bool> = const { Cell::new(false) };
}

/// Unwind payload carrying a `process.exit` code back to [`run_program`].
struct EmbedExit(i32);

/// Called by `process.exit` / `process.abort` before they end the process:
/// on a [`run_program`] thread, unwind to it with `code` instead. Returns
/// (does nothing) everywhere else. `resume_unwind` skips the panic hook, so
/// nothing is printed.
pub(crate) fn exit_if_embedded(code: i32) {
    if EMBEDDED.with(Cell::get) {
        resume_unwind(Box::new(EmbedExit(code)));
    }
}

/// Run `src` as a complete program with `input` as its stdin and return
/// everything it wrote to stdout.
///
/// Each call runs on its own thread with [`crate::JS_STACK_SIZE`] of stack and
/// a freshly reset host, so nothing leaks between calls and concurrent calls
/// never share a buffer. Stdout writes (`console.log`, `process.stdout.write`)
/// are captured through [`crate::host::JsHost::begin_stdout_capture`]; stderr
/// still reaches the process. `process.stdin`, `fs.readFileSync(0)` and
/// `readline` read `input`.
///
/// A compile error, an uncaught exception, a nonzero `process.exitCode` or
/// `process.exit(n)` with n != 0, and a panic inside the runtime return `Err`.
/// `process.exit(0)` returns `Ok` with the output written so far.
pub fn run_program(src: &str, input: &str) -> Result<String, String> {
    let (src, input) = (src.to_owned(), input.to_owned());
    std::thread::Builder::new()
        .name("node-js-embed".into())
        .stack_size(crate::JS_STACK_SIZE)
        .spawn(move || run_captured(&src, &input))
        .map_err(|e| format!("node-js: cannot spawn interpreter thread: {e}"))?
        .join()
        .unwrap_or_else(|p| Err(panic_message(p.as_ref())))
}

fn run_captured(src: &str, input: &str) -> Result<String, String> {
    EMBEDDED.with(|e| e.set(true));
    let run = catch_unwind(AssertUnwindSafe(|| {
        crate::run_on_fresh_host(src, |h| {
            h.set_stdin(input.as_bytes());
            h.begin_stdout_capture();
        })
    }));
    let (result, output, code) = match run {
        Ok((result, output)) => (result, output, crate::exit_code().unwrap_or(0)),
        Err(payload) => match payload.downcast::<EmbedExit>() {
            Ok(exit) => {
                let output = crate::host::with_host(|h| h.end_capture());
                (Ok(fusevm::Value::Undef), output, exit.0)
            }
            Err(p) => return Err(panic_message(p.as_ref())),
        },
    };
    match (result, code) {
        (Err(e), _) => Err(e),
        (Ok(_), 0) => Ok(output),
        (Ok(_), code) => Err(format!("exit status {code}")),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string());
    format!("node-js: interpreter panicked: {msg}")
}
