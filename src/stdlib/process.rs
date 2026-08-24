//! Node `process` global — the subset packages read at load time.
//!
//! Data properties (`process.env`, `process.argv`, `process.platform`, the
//! `stdout`/`stderr` stream stand-ins, …) are served through `constant`;
//! callable members (`process.cwd()`, `process.hrtime()`, the EventEmitter-style
//! `on`/`emit` no-ops, …) through `call`. `process.nextTick` is intentionally NOT
//! handled here — it stays on the core microtask path in `builtins.rs`.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Callable members. `nextTick` is deliberately absent (handled in `builtins`).
pub const METHODS: &[&str] = &[
    "cwd",
    "chdir",
    "exit",
    "hrtime",
    // `process.hrtime.bigint()` is a real member, not just a property that
    // answers `typeof "function"`. It resolves as the qualified builtin
    // `process.hrtime.bigint` (`namespace_property` composes `ns` + `name`),
    // so it has to be declared here for `is_method` to route the call.
    "hrtime.bigint",
    "uptime",
    "memoryUsage",
    "cpuUsage",
    "umask",
    "binding",
    "emit",
    "on",
    "once",
    "off",
    "addListener",
    "removeListener",
    "removeAllListeners",
    "listeners",
    "emitWarning",
    "kill",
    "getuid",
    "getgid",
    "geteuid",
    "getegid",
    "getgroups",
    "setuid",
    "setgid",
    "seteuid",
    "setegid",
    "setgroups",
    "initgroups",
    "ref",
    "unref",
    "abort",
    "getActiveResourcesInfo",
    "resourceUsage",
    "threadCpuUsage",
    "availableMemory",
    "constrainedMemory",
    "getBuiltinModule",
    "openStdin",
    "hasUncaughtExceptionCaptureCallback",
    "setUncaughtExceptionCaptureCallback",
    "addUncaughtExceptionCaptureCallback",
    "execve",
    "reallyExit",
    "loadEnvFile",
    "setSourceMapsEnabled",
];

thread_local! {
    /// The single `process.setUncaughtExceptionCaptureCallback` slot. Stored as a
    /// heap handle (thread-local like the JS heap); read by
    /// `hasUncaughtExceptionCaptureCallback`.
    static UNCAUGHT_CAPTURE: std::cell::RefCell<Option<Value>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether the one-shot "(Use `node --trace-… ...`)" hint has been printed.
/// Node prints it after the FIRST warning only, per process.
static TRACE_HINT_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Port of `internal/process/warning.js` `onWarning`: render a warning to
/// stderr as `(node:PID) [CODE] Name: message`, followed by an optional detail
/// line and the one-time trace hint. Suppressed by `--no-warnings`, and
/// deprecations additionally by `--no-deprecation`.
pub fn emit_warning(name: &str, code: Option<&str>, message: &str, detail: Option<&str>) {
    let argv: Vec<String> = std::env::args().collect();
    let flag = |f: &str| argv.iter().any(|a| a == f);
    let is_deprecation = name == "DeprecationWarning";
    if flag("--no-warnings") || (is_deprecation && flag("--no-deprecation")) {
        return;
    }
    let trace = flag("--trace-warnings") || (is_deprecation && flag("--trace-deprecation"));

    let mut msg = std::format!("(node:{}) ", std::process::id());
    if let Some(c) = code {
        msg.push_str(&std::format!("[{c}] "));
    }
    msg.push_str(&std::format!("{name}: {message}"));
    if let Some(d) = detail {
        msg.push_str(&std::format!("\n{d}"));
    }
    if !trace && !TRACE_HINT_SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let trace_flag = if is_deprecation {
            "--trace-deprecation"
        } else {
            "--trace-warnings"
        };
        msg.push_str(&std::format!(
            "\n(Use `node {trace_flag} ...` to show where the warning was created)"
        ));
    }
    eprintln!("{msg}");
}

/// A `DeprecationWarning` fires at most once per `code` per process, matching
/// the `warned` latches Node keeps at each deprecation site.
pub fn emit_deprecation_warning(code: &str, message: &str) {
    use std::cell::RefCell;
    thread_local! {
        static SEEN: RefCell<std::collections::HashSet<String>> =
            RefCell::new(std::collections::HashSet::new());
    }
    let first = SEEN.with(|s| s.borrow_mut().insert(code.to_string()));
    if first {
        emit_warning("DeprecationWarning", Some(code), message, None);
    }
}

/// A signal NAME (`"SIGKILL"`, case-insensitive) to its number, or `None` if the
/// name is not one this platform defines.
///
/// The numbers come from `libc`, not from a hand-written table: signal numbering
/// differs between macOS and Linux above SIGTERM (`SIGUSR1` is 30 on Darwin and
/// 10 on Linux), so a literal table is only correct on the platform it was
/// written for. Shared with `cluster`'s `worker.kill`.
pub fn signal_number(name: &str) -> Option<libc::c_int> {
    Some(match name.to_uppercase().as_str() {
        "SIGHUP" => libc::SIGHUP,
        "SIGINT" => libc::SIGINT,
        "SIGQUIT" => libc::SIGQUIT,
        "SIGILL" => libc::SIGILL,
        "SIGTRAP" => libc::SIGTRAP,
        "SIGABRT" => libc::SIGABRT,
        "SIGBUS" => libc::SIGBUS,
        "SIGFPE" => libc::SIGFPE,
        "SIGKILL" => libc::SIGKILL,
        "SIGUSR1" => libc::SIGUSR1,
        "SIGSEGV" => libc::SIGSEGV,
        "SIGUSR2" => libc::SIGUSR2,
        "SIGPIPE" => libc::SIGPIPE,
        "SIGALRM" => libc::SIGALRM,
        "SIGTERM" => libc::SIGTERM,
        "SIGCHLD" => libc::SIGCHLD,
        "SIGCONT" => libc::SIGCONT,
        "SIGSTOP" => libc::SIGSTOP,
        "SIGTSTP" => libc::SIGTSTP,
        "SIGWINCH" => libc::SIGWINCH,
        _ => return None,
    })
}

/// `process.emitWarning(warning[, options])` / `(warning[, type[, code]])`.
fn emit_warning_args(args: &[Value]) {
    let message = super::arg_str(args, 0);
    let mut name = "Warning".to_string();
    let mut code: Option<String> = None;
    let mut detail: Option<String> = None;
    match args.get(1) {
        Some(v) if with_host(|h| matches!(h.get(v), Some(JsObj::Object(_)))) => {
            let field = |k: &str| {
                with_host(|h| match h.get(v) {
                    Some(JsObj::Object(p)) => {
                        p.get(k).filter(|x| !h.is_nullish(x)).map(|x| h.str_of(x))
                    }
                    _ => None,
                })
            };
            if let Some(t) = field("type") {
                name = t;
            }
            code = field("code");
            detail = field("detail");
        }
        Some(_) => {
            name = super::arg_str(args, 1);
            code = args.get(2).map(|_| super::arg_str(args, 2));
        }
        None => {}
    }
    emit_warning(&name, code.as_deref(), &message, detail.as_deref());
}

/// Data properties, served through `namespace_property` → `stdlib::constant`.
/// Memoize an OBJECT-valued `process` property for the host's lifetime.
///
/// `process.env`, `process.argv` and the std streams were rebuilt on every read,
/// so `process.env === process.env` was `false` and — much worse —
/// `process.env.NODE_ENV = "production"` wrote to a throwaway object and read
/// back `undefined`. Node hands out one object per property, and packages both
/// mutate it and compare it by identity.
///
/// `builtin_static` is the existing side table for exactly this: state that must
/// survive the fresh `Builtin` handle each `process` reference allocates. It is
/// per-host, so `reset_host` clears it and a stale handle cannot outlive its heap.
fn memo(name: &str, make: impl FnOnce() -> Value) -> Value {
    if let Some(v) = with_host(|h| h.builtin_static("process", name)) {
        return v;
    }
    let v = make();
    with_host(|h| h.set_builtin_static("process", name, v.clone()));
    v
}

pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "env" => memo("env", env_object),
        "argv" => memo("argv", argv),
        "argv0" => with_host(|h| h.new_str(exec_path())),
        "execPath" => with_host(|h| h.new_str(exec_path())),
        "execArgv" => memo("execArgv", exec_argv),
        "platform" => with_host(|h| h.new_str(super::os::platform())),
        "arch" => with_host(|h| h.new_str(super::os::arch())),
        "pid" => Value::Float(std::process::id() as f64),
        "ppid" => Value::Float(0.0),
        "title" => with_host(|h| h.new_str("node")),
        // A best-effort Node-compatible version string. Kept low so a dep's
        // `if (semver.lt(process.version, ...))` gate takes the conservative path.
        "version" => with_host(|h| h.new_str("v26.5.0")),
        "versions" => memo("versions", versions),
        "stdout" => memo("stdout", || std_stream(1)),
        "stderr" => memo("stderr", || std_stream(2)),
        "stdin" => memo("stdin", || std_stream(0)),
        // Unset reads back as `undefined`, not `0` — `process.exitCode` starts
        // life absent and a script may test for that.
        "exitCode" => match with_host(|h| h.exit_code) {
            Some(c) => Value::Float(c as f64),
            None => Value::Undef,
        },
        _ => return None,
    })
}

/// The `process.exitCode` setter, ported from Node's
/// `lib/internal/bootstrap/node.js` accessor:
///
/// ```js
/// set(code) {
///   if (code !== null && code !== undefined) {
///     let value = code;
///     if (typeof code === 'string' && code !== '' &&
///       NumberIsNaN((value = Number(code)))) {
///       value = code;
///     }
///     validateInteger(value, 'code');
///     …
///   } else { /* clear */ }
/// }
/// ```
///
/// So a NUMERIC string is accepted and coerced (`"3"` → 3, `"0x10"` → 16,
/// `"  "` → 0), a non-numeric or empty string keeps its string identity and
/// fails `validateInteger` as a TYPE error, a non-integer number fails as a
/// RANGE error, and `null`/`undefined` clear the slot. Verified on node
/// v26.7.0: `process.exitCode = "0x10"` exits 16, `= 3.7` throws
/// `ERR_OUT_OF_RANGE`, `= ""` throws `ERR_INVALID_ARG_TYPE`, `= "  "` exits 0.
pub fn set_exit_code(val: &Value) -> Result<(), String> {
    if matches!(val, Value::Undef) || with_host(|h| h.is_null(val)) {
        with_host(|h| h.exit_code = None);
        return Ok(());
    }
    // A numeric string coerces; anything else keeps its own type for the error.
    let numeric = match with_host(|h| h.as_str(val)) {
        Some(s) if !s.is_empty() => {
            let n = with_host(|h| h.to_number(val));
            if n.is_nan() {
                None
            } else {
                Some(n)
            }
        }
        Some(_) => None,
        None => match val {
            Value::Float(_) | Value::Int(_) => Some(with_host(|h| h.to_number(val))),
            _ => None,
        },
    };
    match numeric {
        Some(n) if n.fract() == 0.0 && n.is_finite() => {
            with_host(|h| h.exit_code = Some(n as i32));
            Ok(())
        }
        Some(n) => Err(crate::host::coded_error(
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"code\" is out of range. It must be an integer. Received {}",
                crate::host::fmt_number(n)
            ),
        )),
        None => Err(crate::host::invalid_arg_type(
            "code", "argument", "number", val,
        )),
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "cwd" => {
            let d = std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(with_host(|h| h.new_str(d)))
        }
        // `hrtime()` → `[seconds, nanoseconds]` since an arbitrary epoch (here the
        // monotonic clock via `Instant` is unavailable statically, so use the
        // system clock — sufficient for the timing scaffolding deps set up).
        "hrtime" => Ok(hrtime(args)),
        // Nanoseconds since an arbitrary epoch, as a BigInt.
        "hrtime.bigint" => Ok(with_host(|h| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            h.new_bigint(num_bigint::BigInt::from(now.as_nanos()))
        })),
        "uptime" => Ok(Value::Float(0.0)),
        "memoryUsage" => Ok(memory_usage()),
        "cpuUsage" => Ok(with_host(|h| {
            let mut m = IndexMap::new();
            m.insert("user".into(), Value::Float(0.0));
            m.insert("system".into(), Value::Float(0.0));
            h.new_object(m)
        })),
        "umask" => Ok(Value::Float(0.0)),
        "binding" => Err(crate::host::type_error("process.binding is not supported")),
        // EventEmitter-style registration. Listeners are REMEMBERED (the runtime
        // emits `unhandledRejection`; signals still never fire), and every form
        // returns the process namespace so `.on(...).on(...)` chains work.
        "on" | "once" | "addListener" => {
            let (event, f) = (event_name(args), args.get(1).cloned());
            if let Some(f) = f {
                let once = method == "once";
                with_host(|h| {
                    h.process_listeners
                        .entry(event)
                        .or_default()
                        .push(crate::host::ProcListener { f, once })
                });
            }
            Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into()))))
        }
        "off" | "removeListener" => {
            let (event, f) = (event_name(args), args.get(1).cloned());
            if let Some(f) = f {
                with_host(|h| {
                    if let Some(l) = h.process_listeners.get_mut(&event) {
                        if let Some(i) = l.iter().position(|x| x.f == f) {
                            l.remove(i);
                        }
                    }
                });
            }
            Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into()))))
        }
        "removeAllListeners" => {
            let event = event_name(args);
            with_host(|h| {
                if event.is_empty() {
                    h.process_listeners.clear();
                } else {
                    h.process_listeners.shift_remove(&event);
                }
            });
            Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into()))))
        }
        "listeners" => {
            let event = event_name(args);
            Ok(with_host(|h| {
                let l = h
                    .process_listeners
                    .get(&event)
                    .map(|v| v.iter().map(|x| x.f.clone()).collect())
                    .unwrap_or_default();
                h.new_array(l)
            }))
        }
        "emit" => {
            let event = event_name(args);
            let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
            let listeners = with_host(|h| h.take_process_listeners(&event));
            let any = !listeners.is_empty();
            let mut r = Ok(Value::Bool(any));
            for f in listeners {
                if let Err(e) = crate::host::invoke(&f, rest.clone(), None) {
                    r = Err(e);
                    break;
                }
            }
            r
        }
        "emitWarning" => {
            emit_warning_args(args);
            Ok(Value::Undef)
        }
        // `process.exit([code])` really exits, and does so IMMEDIATELY — nothing
        // after the call runs. It used to return `undefined` and let execution
        // continue, which is a silent lie with teeth: the idiom
        // `if (done) { server.close(); process.exit(0); }` (no `return`, because
        // in Node none is needed) fell through to the statement after it. In a
        // request-sequencing loop that meant re-entering the loop past the end of
        // its array and destructuring `undefined`. Measured on node v26.7.0,
        // `console.log('before'); process.exit(0); console.log('after')` prints
        // only `before`; it printed both here.
        //
        // Under `--build`/`--dap`/embedding this is still a real process exit,
        // exactly as it is in Node — there is no "exit but keep going" in the API.
        // stdout/stderr are flushed first because `std::process::exit` runs no
        // destructors.
        //
        // Port of Node's `process.exit`: an argument (even `undefined`) is
        // ASSIGNED to `process.exitCode` first — through the validating setter,
        // so `process.exit(3.7)` throws instead of exiting — then the `exit`
        // event fires with the resulting code, then the process leaves. With no
        // argument the already-set `process.exitCode` decides, which is why
        // `process.exitCode = 3; process.exit()` exits 3 on node v26.7.0.
        "exit" | "reallyExit" => {
            if !args.is_empty() {
                if let Err(e) = set_exit_code(&args[0]) {
                    return Some(Err(e));
                }
            }
            let code = with_host(|h| h.exit_code).unwrap_or(0);
            if let Err(e) = emit_exit_event(code) {
                return Some(Err(e));
            }
            // An `exit` listener may raise the code; re-read before leaving.
            let code = with_host(|h| h.exit_code).unwrap_or(0);
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            // `std::process::exit` runs no destructors, so the bytecode cache
            // has to reach disk here too — otherwise a script that ends in
            // `process.exit()` would recompile every module it loaded, every
            // run, and never benefit from the cache at all.
            crate::cache::flush();
            std::process::exit(code);
        }
        // `process.chdir(dir)` really changes the working directory, and throws on
        // failure; it used to silently do nothing, so every later relative path
        // still resolved against the old directory.
        "chdir" => {
            let dir = super::arg_str(args, 0);
            std::env::set_current_dir(&dir)
                .map(|()| Value::Undef)
                // Node reports the libuv message and BOTH directories:
                // `ENOENT: no such file or directory, chdir <cwd> -> <dir>`.
                // The old text spliced in Rust's `io::Error` Display, whose
                // `No such file or directory (os error 2)` no Node ever printed.
                .map_err(|e| {
                    let from = std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    format!(
                        "Error: {}, chdir '{from}' -> '{dir}'",
                        crate::stdlib::fs::libuv_message(&e)
                    )
                })
        }
        // `process.kill(pid[, signal])` really signals the process. Node's default
        // is SIGTERM, and a numeric or `'SIGxxx'` signal is accepted; signal `0`
        // is the existence probe and sends nothing.
        "kill" => {
            let pid = with_host(|h| args.first().map(|v| h.to_number(v)).unwrap_or(0.0)) as i32;
            let sig: Result<libc::c_int, String> = match args.get(1) {
                Some(v) if !matches!(v, Value::Undef) => match with_host(|h| h.as_str(v)) {
                    Some(name) => signal_number(&name).ok_or(crate::host::coded_error(
                        "TypeError",
                        "ERR_UNKNOWN_SIGNAL",
                        &format!("Unknown signal: {name}"),
                    )),
                    None => Ok(with_host(|h| h.to_number(v)) as libc::c_int),
                },
                _ => Ok(libc::SIGTERM),
            };
            sig.and_then(|sig| {
                // SAFETY: `kill` is a plain syscall on a pid/signal pair; it
                // mutates no process memory and reports failure through `errno`.
                if unsafe { libc::kill(pid, sig) } != 0 {
                    Err(format!("Error: {}", std::io::Error::last_os_error()))
                } else {
                    Ok(Value::Undef)
                }
            })
        }
        // A genuine no-op: node-js emits no source maps, so enabling their use
        // changes nothing. Returning `undefined` is the whole of Node's contract
        // here, so this is not a stub.
        "setSourceMapsEnabled" => Ok(Value::Undef),

        // POSIX identity queries (libc; pure reads, always safe).
        "getuid" => Ok(Value::Float(unsafe { libc::getuid() } as f64)),
        "geteuid" => Ok(Value::Float(unsafe { libc::geteuid() } as f64)),
        "getgid" => Ok(Value::Float(unsafe { libc::getgid() } as f64)),
        "getegid" => Ok(Value::Float(unsafe { libc::getegid() } as f64)),
        "getgroups" => {
            let groups = supplementary_groups();
            Ok(with_host(|h| {
                h.new_array(groups.into_iter().map(Value::Float).collect())
            }))
        }

        // POSIX identity mutation (libc; best-effort — silently ignored when the
        // process lacks the privilege, matching a no-throw best-effort surface).
        "setuid" | "seteuid" | "setgid" | "setegid" => {
            let id = super::arg_num(args, 0);
            if id.is_finite() {
                let id = id as u32;
                // SAFETY: id is a plain uid/gid number; a failed call just returns -1.
                unsafe {
                    match method {
                        "setuid" => libc::setuid(id),
                        "seteuid" => libc::seteuid(id),
                        "setgid" => libc::setgid(id),
                        _ => libc::setegid(id),
                    };
                }
            }
            Ok(Value::Undef)
        }
        "setgroups" => {
            let groups = gid_array(args.first());
            // SAFETY: `groups` is a valid gid buffer of the given length.
            unsafe {
                libc::setgroups(groups.len() as _, groups.as_ptr());
            }
            Ok(Value::Undef)
        }
        "initgroups" => {
            let user = super::arg_str(args, 0);
            let extra = super::arg_num(args, 1);
            if let Ok(c) = std::ffi::CString::new(user) {
                let gid = if extra.is_finite() { extra as u32 } else { 0 };
                // SAFETY: `c` is NUL-terminated; a failed call just returns -1.
                unsafe {
                    libc::initgroups(c.as_ptr(), gid as _);
                }
            }
            Ok(Value::Undef)
        }

        // `ref`/`unref` on the process object are chainable no-ops (no libuv
        // handle refcount to touch); return the process namespace.
        "ref" | "unref" => Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into())))),
        "abort" => std::process::abort(),
        "getActiveResourcesInfo" => Ok(with_host(|h| h.new_array(Vec::new()))),
        "resourceUsage" => Ok(resource_usage()),
        "threadCpuUsage" => Ok(thread_cpu_usage()),
        "availableMemory" | "constrainedMemory" => Ok(Value::Float(0.0)),
        "getBuiltinModule" => {
            let id = super::arg_str(args, 0);
            let id = id.strip_prefix("node:").unwrap_or(&id);
            match crate::stdlib::resolve(id) {
                Some(ns) => Ok(with_host(|h| h.alloc(JsObj::Builtin(ns.to_string())))),
                None => Ok(Value::Undef),
            }
        }
        "openStdin" => Ok(std_stream(0)),

        "hasUncaughtExceptionCaptureCallback" => {
            Ok(Value::Bool(UNCAUGHT_CAPTURE.with(|c| c.borrow().is_some())))
        }
        "setUncaughtExceptionCaptureCallback" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            let clear = matches!(cb, Value::Undef) || with_host(|h| h.is_null(&cb));
            if clear {
                UNCAUGHT_CAPTURE.with(|c| *c.borrow_mut() = None);
            } else if UNCAUGHT_CAPTURE.with(|c| c.borrow().is_some()) {
                return Some(Err(crate::host::type_error(
                    "`process.setUncaughtExceptionCaptureCallback()` was called \
                     while a capture callback was already active",
                )));
            } else {
                UNCAUGHT_CAPTURE.with(|c| *c.borrow_mut() = Some(cb));
            }
            Ok(Value::Undef)
        }
        "addUncaughtExceptionCaptureCallback" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            if !matches!(cb, Value::Undef) {
                UNCAUGHT_CAPTURE.with(|c| *c.borrow_mut() = Some(cb));
            }
            Ok(Value::Undef)
        }
        "execve" => exec_ve(args),
        "loadEnvFile" => load_env_file(&super::arg_str(args, 0)),
        _ => return None,
    })
}

/// `process.env` as a plain object built from the real environment.
fn env_object() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for (k, v) in std::env::vars() {
            m.insert(k, h.new_str(v));
        }
        h.new_object(m)
    })
}

/// The `(execArgv, argv)` split installed by the binary's entry point.
///
/// Unset when the library is embedded (or driven by a sibling binary such as
/// `parity-fuzz`, whose own command line is not a `node` command line), and the
/// accessors below then fall back to the raw process arguments.
static ARGV: std::sync::OnceLock<(Vec<String>, Vec<String>)> = std::sync::OnceLock::new();

/// Publish Node's `process.argv` / `process.execArgv` split for this run, read
/// off the real command line. Called once by the `node` binary's entry point;
/// first call wins.
///
/// `argv[1]` is the entry script RESOLVED against the current directory, so
/// `node ./x.js` reports the same absolute path `path.resolve` would — not the
/// spelling that was typed.
pub fn install_argv() {
    let split = crate::cli::split_argv(std::env::args());
    let mut argv = vec![exec_path()];
    if let Some(s) = &split.script {
        // `-` is Node's stdin entry point, not a path; it stays verbatim.
        argv.push(if s == "-" {
            s.clone()
        } else {
            super::path::resolve_one(s)
        });
    }
    argv.extend(split.user);
    let _ = ARGV.set((split.exec, argv));
}

/// `process.argv`: `[execPath, entryScript, ...userArgs]`.
///
/// The runtime's OWN flags are not in it — they are `process.execArgv` — and
/// under `-e` there is no `argv[1]` at all. Returning the raw OS arguments put
/// `-e` and the whole one-liner source into `argv`, which is what any script
/// that reads `process.argv.slice(2)` for its options would have parsed.
fn argv() -> Value {
    with_host(|h| {
        let items: Vec<Value> = match ARGV.get() {
            Some((_, argv)) => argv.iter().map(|a| h.new_str(a.clone())).collect(),
            None => std::env::args().map(|a| h.new_str(a)).collect(),
        };
        h.new_array(items)
    })
}

/// `process.execArgv`: the runtime flags, `-e`/`--eval` and its source included.
fn exec_argv() -> Value {
    with_host(|h| {
        let items: Vec<Value> = ARGV
            .get()
            .map(|(e, _)| e.iter().map(|a| h.new_str(a.clone())).collect())
            .unwrap_or_default();
        h.new_array(items)
    })
}

fn exec_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "node".into())
}

/// `process.versions` — a small map; only `node` is commonly gated on.
fn versions() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("node".into(), h.new_str("26.5.0"));
        m.insert("v8".into(), h.new_str("0.0.0"));
        h.new_object(m)
    })
}

/// A minimal `process.stdout`/`stderr`/`stdin` stand-in: enough surface
/// (`fd`, `isTTY`, `writable`, a `write`) for load-time probes like
/// `tty.isatty(process.stderr.fd)`.
fn std_stream(fd: i32) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("WriteStream"));
        m.insert("fd".into(), Value::Float(fd as f64));
        // SAFETY: isatty is a pure query on the fd number.
        let is_tty = unsafe { libc::isatty(fd) == 1 };
        m.insert("isTTY".into(), Value::Bool(is_tty));
        m.insert("writable".into(), Value::Bool(fd != 0));
        m.insert("readable".into(), Value::Bool(fd == 0));
        // A tty stream exposes its terminal dimensions (real ioctl reading).
        if is_tty {
            if let Some((cols, rows)) = super::tty::window_size(fd) {
                m.insert("columns".into(), Value::Float(cols as f64));
                m.insert("rows".into(), Value::Float(rows as f64));
            }
        }
        h.new_object(m)
    })
}

/// Instance methods of a `process.stdout`/`stderr` `WriteStream`: `write`/`end`
/// emit the chunk raw (no newline) to the stream's fd, so ordering interleaves
/// correctly with `console.log`.
pub fn stream_instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "write" | "end" => {
            let fd = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p.get("fd").map(|v| h.to_number(v)).unwrap_or(1.0),
                _ => 1.0,
            });
            // `end()` with no chunk closes without writing; `write()` with no
            // chunk is the argument error below.
            if method == "end" && args.first().map(|v| matches!(v, Value::Undef)) != Some(false) {
                return Ok(Value::Bool(true));
            }
            let bytes = chunk_bytes(args)?;
            with_host(|h| h.write_out_bytes(&bytes, fd == 2.0));
            Ok(Value::Bool(true))
        }
        // A no-op stream surface so `.on('data')`/`.once`/`.end()` chaining loads.
        "on" | "once" | "removeListener" | "cork" | "uncork" | "setEncoding" => Ok(recv.clone()),
        // `tty.WriteStream` cursor/erase control — emit the corresponding ANSI
        // escape to the stream's fd (best-effort; only meaningful on a real tty).
        "cursorTo" | "moveCursor" | "clearLine" | "clearScreenDown" => {
            let seq = tty_control(method, args);
            write_fd(stream_fd(recv), seq.as_bytes());
            Ok(Value::Bool(true))
        }
        "getWindowSize" => {
            let (c, r) = super::tty::window_size(stream_fd(recv) as i32).unwrap_or((80, 24));
            Ok(with_host(|h| {
                h.new_array(vec![Value::Float(c as f64), Value::Float(r as f64)])
            }))
        }
        // A truecolor terminal advertises 24-bit depth; hasColors(count) is true
        // for any request within that range.
        "getColorDepth" => Ok(Value::Float(24.0)),
        "hasColors" => Ok(Value::Bool(true)),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

fn hrtime(args: &[Value]) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (mut secs, mut nanos) = (now.as_secs() as f64, now.subsec_nanos() as f64);
    // `hrtime(prev)` returns the diff from a prior reading.
    if let Some(Value::Obj(_)) = args.first() {
        if let Some(prev) = with_host(|h| match h.get(&args[0]) {
            Some(JsObj::Array(a)) if a.len() == 2 => Some((h.to_number(&a[0]), h.to_number(&a[1]))),
            _ => None,
        }) {
            secs -= prev.0;
            nanos -= prev.1;
        }
    }
    with_host(|h| h.new_array(vec![Value::Float(secs), Value::Float(nanos)]))
}

fn memory_usage() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        for k in ["rss", "heapTotal", "heapUsed", "external", "arrayBuffers"] {
            m.insert(k.into(), Value::Float(0.0));
        }
        h.new_object(m)
    })
}

/// Emit `process.on('exit', code)` exactly once per process, the way Node's
/// `process._exiting` latch does — `process.exit()` inside an `exit` handler
/// must not re-enter it.
///
/// The handlers run SYNCHRONOUSLY and nothing they schedule ever runs: Node
/// leaves the loop straight after them, so a `setTimeout` or `.then` queued
/// here is dropped. An `exit` listener may still raise `process.exitCode`, and
/// that later value is the one the process uses, which is why the caller reads
/// the slot back after this returns.
pub fn emit_exit_event(code: i32) -> Result<(), String> {
    if with_host(|h| std::mem::replace(&mut h.exiting, true)) {
        return Ok(());
    }
    let listeners = with_host(|h| h.take_process_listeners("exit"));
    for f in listeners {
        crate::host::invoke(&f, vec![Value::Float(code as f64)], None)?;
    }
    Ok(())
}

/// Emit `process.on('beforeExit', code)`. Node fires this when the loop has
/// drained but the process has NOT been told to exit, and — unlike `exit` —
/// work scheduled from a handler is honoured, so the loop runs again and
/// `beforeExit` can fire repeatedly. It never fires after an explicit
/// `process.exit()` or an uncaught exception.
///
/// Reports whether any listener ran, so the caller knows to re-drain.
pub fn emit_before_exit(code: i32) -> Result<bool, String> {
    let listeners = with_host(|h| h.take_process_listeners("beforeExit"));
    let any = !listeners.is_empty();
    for f in listeners {
        crate::host::invoke(&f, vec![Value::Float(code as f64)], None)?;
    }
    Ok(any)
}

/// The bytes a `stream.write(chunk[, encoding])` call puts on the wire.
///
/// Node writes a `Buffer`/`TypedArray`/`DataView` chunk through UNTOUCHED, and
/// decodes a string chunk with the named encoding (default `utf8`). Both were
/// funnelled through `ToString` here, which is lossy in two separate ways:
/// `process.stdout.write(Buffer.from([0xff,0xfe,0x41]))` printed the 15 bytes of
/// `[object Object]` instead of `ff fe 41`, and even once the Buffer path
/// existed, a `String` round-trip would have replaced each non-UTF-8 byte with
/// `U+FFFD` (3 bytes out, 7 bytes on the wire). `write("4142","hex")` likewise
/// printed the four characters of the literal instead of the two bytes `AB`.
///
/// Anything that is neither a string nor a byte view is the same
/// `ERR_INVALID_ARG_TYPE` Node raises — a JS array of byte values included,
/// which is why this does NOT reuse `buffer::bytes_like` (that helper
/// deliberately accepts plain arrays, which `write` rejects).
fn chunk_bytes(args: &[Value]) -> Result<Vec<u8>, String> {
    let chunk = args.first().cloned().unwrap_or(Value::Undef);
    if with_host(|h| h.is_null(&chunk)) {
        return Err(crate::host::type_error(
            "May not write null values to stream",
        ));
    }
    if let Some(s) = with_host(|h| h.as_str(&chunk)) {
        let enc = match args.get(1) {
            Some(v) if !matches!(v, Value::Undef) => with_host(|h| h.str_of(v)),
            _ => "utf8".to_string(),
        };
        return Ok(super::buffer::decode_str(&s, &enc));
    }
    match super::native_tag(&chunk).as_deref() {
        Some("Buffer") | Some("TypedArray") | Some("DataView") => {
            Ok(super::buffer::bytes_like(&chunk).unwrap_or_default())
        }
        _ => Err(crate::host::type_error(&format!(
            "The \"chunk\" argument must be of type string or an instance of \
             Buffer, TypedArray, or DataView. Received {}",
            super::received_desc(&chunk)
        ))),
    }
}

/// The `fd` numeric property of a stream stand-in (default stdout).
fn stream_fd(recv: &Value) -> f64 {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("fd").map(|v| h.to_number(v)).unwrap_or(1.0),
        _ => 1.0,
    })
}

/// Write raw bytes as program output on stdout/stderr (chosen by fd) — through
/// the host funnel, so an embedder capturing output receives these too.
fn write_fd(fd: f64, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    with_host(|h| h.write_out(&text, fd == 2.0));
}

/// The ANSI control sequence for a `tty.WriteStream` cursor/erase method.
fn tty_control(method: &str, args: &[Value]) -> String {
    match method {
        // cursorTo(x[, y]) → absolute column (`\e[<x+1>G`) or position.
        "cursorTo" => {
            let x = super::arg_num(args, 0);
            let y = super::arg_num(args, 1);
            let x = if x.is_finite() { x as i64 } else { 0 };
            if y.is_finite() {
                format!("\x1b[{};{}H", y as i64 + 1, x + 1)
            } else {
                format!("\x1b[{}G", x + 1)
            }
        }
        // moveCursor(dx, dy) → relative moves.
        "moveCursor" => {
            let dx = super::arg_num(args, 0);
            let dy = super::arg_num(args, 1);
            let mut s = String::new();
            let dx = if dx.is_finite() { dx as i64 } else { 0 };
            let dy = if dy.is_finite() { dy as i64 } else { 0 };
            if dx > 0 {
                s.push_str(&format!("\x1b[{dx}C"));
            } else if dx < 0 {
                s.push_str(&format!("\x1b[{}D", -dx));
            }
            if dy > 0 {
                s.push_str(&format!("\x1b[{dy}B"));
            } else if dy < 0 {
                s.push_str(&format!("\x1b[{}A", -dy));
            }
            s
        }
        // clearLine(dir): -1 left, 1 right, 0 whole line.
        "clearLine" => match super::arg_num(args, 0) {
            d if d < 0.0 => "\x1b[1K".into(),
            d if d > 0.0 => "\x1b[0K".into(),
            _ => "\x1b[2K".into(),
        },
        // clearScreenDown → erase from cursor to end of screen.
        _ => "\x1b[0J".into(),
    }
}

/// The process's supplementary group ids (`getgroups(2)`).
fn supplementary_groups() -> Vec<f64> {
    // SAFETY: first call queries the count, second fills a buffer of that size.
    unsafe {
        let n = libc::getgroups(0, std::ptr::null_mut());
        if n <= 0 {
            return Vec::new();
        }
        let mut buf = vec![0 as libc::gid_t; n as usize];
        let filled = libc::getgroups(n, buf.as_mut_ptr());
        if filled < 0 {
            return Vec::new();
        }
        buf.truncate(filled as usize);
        buf.into_iter().map(|g| g as f64).collect()
    }
}

/// Read a JS array of numbers as a gid buffer.
fn gid_array(v: Option<&Value>) -> Vec<libc::gid_t> {
    let Some(v) = v else { return Vec::new() };
    with_host(|h| match h.get(v) {
        Some(JsObj::Array(a)) => a.iter().map(|x| h.to_number(x) as libc::gid_t).collect(),
        _ => Vec::new(),
    })
}

/// `getrusage(RUSAGE_SELF)` — `None` if the syscall fails.
fn get_rusage() -> Option<libc::rusage> {
    // SAFETY: getrusage fills a zeroed rusage; RUSAGE_SELF is a valid `who`.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut ru) == 0).then_some(ru)
    }
}

/// microseconds from a `timeval`.
fn tv_micros(t: &libc::timeval) -> f64 {
    t.tv_sec as f64 * 1e6 + t.tv_usec as f64
}

/// `process.resourceUsage()` — the full `getrusage` breakdown (zeros on failure).
fn resource_usage() -> Value {
    let ru = get_rusage();
    with_host(|h| {
        let mut m = IndexMap::new();
        let (utime, stime) = ru
            .as_ref()
            .map(|r| (tv_micros(&r.ru_utime), tv_micros(&r.ru_stime)))
            .unwrap_or((0.0, 0.0));
        m.insert("userCPUTime".into(), Value::Float(utime));
        m.insert("systemCPUTime".into(), Value::Float(stime));
        let fields = [
            ("maxRSS", ru.as_ref().map(|r| r.ru_maxrss)),
            ("sharedMemorySize", ru.as_ref().map(|r| r.ru_ixrss)),
            ("unsharedDataSize", ru.as_ref().map(|r| r.ru_idrss)),
            ("unsharedStackSize", ru.as_ref().map(|r| r.ru_isrss)),
            ("minorPageFault", ru.as_ref().map(|r| r.ru_minflt)),
            ("majorPageFault", ru.as_ref().map(|r| r.ru_majflt)),
            ("swappedOut", ru.as_ref().map(|r| r.ru_nswap)),
            ("fsRead", ru.as_ref().map(|r| r.ru_inblock)),
            ("fsWrite", ru.as_ref().map(|r| r.ru_oublock)),
            ("ipcSent", ru.as_ref().map(|r| r.ru_msgsnd)),
            ("ipcReceived", ru.as_ref().map(|r| r.ru_msgrcv)),
            ("signalsCount", ru.as_ref().map(|r| r.ru_nsignals)),
            ("voluntaryContextSwitches", ru.as_ref().map(|r| r.ru_nvcsw)),
            (
                "involuntaryContextSwitches",
                ru.as_ref().map(|r| r.ru_nivcsw),
            ),
        ];
        for (k, v) in fields {
            m.insert(k.into(), Value::Float(v.unwrap_or(0) as f64));
        }
        h.new_object(m)
    })
}

/// `process.threadCpuUsage()` — best-effort via process-wide `getrusage` (no
/// per-thread accounting substrate), reported as `{user, system}` microseconds.
fn thread_cpu_usage() -> Value {
    let (u, s) = get_rusage()
        .map(|r| (tv_micros(&r.ru_utime), tv_micros(&r.ru_stime)))
        .unwrap_or((0.0, 0.0));
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("user".into(), Value::Float(u));
        m.insert("system".into(), Value::Float(s));
        h.new_object(m)
    })
}

/// `process.execve(file, args[, env])` — replace the process image (never returns
/// on success; throws the OS error otherwise).
fn exec_ve(args: &[Value]) -> Result<Value, String> {
    use std::ffi::CString;
    let prog = CString::new(super::arg_str(args, 0))
        .map_err(|_| crate::host::type_error("process.execve: invalid file path"))?;

    let argv_strs: Vec<String> = with_host(|h| match args.get(1).and_then(|v| h.get(v)) {
        Some(JsObj::Array(a)) => a.iter().map(|x| h.str_of(x)).collect(),
        _ => Vec::new(),
    });
    let env_strs: Vec<String> = {
        let from_arg = with_host(|h| match args.get(2).and_then(|v| h.get(v)) {
            Some(JsObj::Object(p)) => Some(
                p.iter()
                    .map(|(k, v)| format!("{k}={}", h.str_of(v)))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        });
        from_arg.unwrap_or_else(|| std::env::vars().map(|(k, v)| format!("{k}={v}")).collect())
    };

    let to_c = |s: String| {
        CString::new(s).map_err(|_| crate::host::type_error("process.execve: NUL in argument"))
    };
    let argv_c: Vec<CString> = argv_strs.into_iter().map(to_c).collect::<Result<_, _>>()?;
    let env_c: Vec<CString> = env_strs.into_iter().map(to_c).collect::<Result<_, _>>()?;

    let mut argv_p: Vec<*const libc::c_char> = argv_c.iter().map(|c| c.as_ptr()).collect();
    argv_p.push(std::ptr::null());
    let mut envp_p: Vec<*const libc::c_char> = env_c.iter().map(|c| c.as_ptr()).collect();
    envp_p.push(std::ptr::null());

    // SAFETY: argv/envp are NUL-terminated arrays of valid C strings kept alive
    // above; on success execve never returns.
    unsafe {
        libc::execve(prog.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr());
    }
    Err(crate::host::type_error(&format!(
        "process.execve failed: {}",
        std::io::Error::last_os_error()
    )))
}

/// `process.loadEnvFile([path])` — parse a `.env` file into `process.env`
/// (persisted through the real environment so a later `process.env` read sees it).
fn load_env_file(path: &str) -> Result<Value, String> {
    let path = if path.is_empty() { ".env" } else { path };
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("Error: ENOENT: {e}, open '{path}'"))?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut val = val.trim();
        if val.len() >= 2
            && ((val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\'')))
        {
            val = &val[1..val.len() - 1];
        }
        std::env::set_var(key, val);
    }
    Ok(Value::Undef)
}

/// The event name argument of an EventEmitter-style `process` call.
fn event_name(args: &[Value]) -> String {
    args.first()
        .map(|v| with_host(|h| h.str_of(v)))
        .unwrap_or_default()
}
