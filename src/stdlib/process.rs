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

/// Data properties, served through `namespace_property` → `stdlib::constant`.
pub fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "env" => env_object(),
        "argv" => argv(),
        "argv0" => with_host(|h| h.new_str(exec_path())),
        "execPath" => with_host(|h| h.new_str(exec_path())),
        "execArgv" => with_host(|h| h.new_array(Vec::new())),
        "platform" => with_host(|h| h.new_str(super::os::platform())),
        "arch" => with_host(|h| h.new_str(super::os::arch())),
        "pid" => Value::Float(std::process::id() as f64),
        "ppid" => Value::Float(0.0),
        "title" => with_host(|h| h.new_str("node")),
        // A best-effort Node-compatible version string. Kept low so a dep's
        // `if (semver.lt(process.version, ...))` gate takes the conservative path.
        "version" => with_host(|h| h.new_str("v26.5.0")),
        "versions" => versions(),
        "stdout" => std_stream(1),
        "stderr" => std_stream(2),
        "stdin" => std_stream(0),
        _ => return None,
    })
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "cwd" => {
            let d = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(with_host(|h| h.new_str(d)))
        }
        // `hrtime()` → `[seconds, nanoseconds]` since an arbitrary epoch (here the
        // monotonic clock via `Instant` is unavailable statically, so use the
        // system clock — sufficient for the timing scaffolding deps set up).
        "hrtime" => Ok(hrtime(args)),
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
        // EventEmitter-style registration is accepted and ignored (returns the
        // process namespace so `.on(...).on(...)` chains work); no signals fire.
        "on" | "once" | "off" | "addListener" | "removeListener" | "removeAllListeners" => {
            Ok(with_host(|h| h.alloc(JsObj::Builtin("process".into()))))
        }
        "listeners" => Ok(with_host(|h| h.new_array(Vec::new()))),
        "emit" => Ok(Value::Bool(false)),
        "emitWarning" => Ok(Value::Undef),
        "chdir" | "exit" | "kill" | "reallyExit" | "setSourceMapsEnabled" => Ok(Value::Undef),

        // POSIX identity queries (libc; pure reads, always safe).
        "getuid" => Ok(Value::Float(unsafe { libc::getuid() } as f64)),
        "geteuid" => Ok(Value::Float(unsafe { libc::geteuid() } as f64)),
        "getgid" => Ok(Value::Float(unsafe { libc::getgid() } as f64)),
        "getegid" => Ok(Value::Float(unsafe { libc::getegid() } as f64)),
        "getgroups" => {
            let groups = supplementary_groups();
            Ok(with_host(|h| h.new_array(groups.into_iter().map(Value::Float).collect())))
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
            unsafe { libc::setgroups(groups.len() as _, groups.as_ptr()); }
            Ok(Value::Undef)
        }
        "initgroups" => {
            let user = super::arg_str(args, 0);
            let extra = super::arg_num(args, 1);
            if let Ok(c) = std::ffi::CString::new(user) {
                let gid = if extra.is_finite() { extra as u32 } else { 0 };
                // SAFETY: `c` is NUL-terminated; a failed call just returns -1.
                unsafe { libc::initgroups(c.as_ptr(), gid as _); }
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

/// `process.argv`: `[execPath, entryScript, ...userArgs]` (best effort).
fn argv() -> Value {
    with_host(|h| {
        let items: Vec<Value> = std::env::args().map(|a| h.new_str(a)).collect();
        h.new_array(items)
    })
}

fn exec_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "node".into())
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
            let chunk = super::arg_str(args, 0);
            use std::io::Write as _;
            if fd == 2.0 {
                let _ = std::io::stderr().write_all(chunk.as_bytes());
                let _ = std::io::stderr().flush();
            } else {
                let _ = std::io::stdout().write_all(chunk.as_bytes());
                let _ = std::io::stdout().flush();
            }
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
            Ok(with_host(|h| h.new_array(vec![Value::Float(c as f64), Value::Float(r as f64)])))
        }
        // A truecolor terminal advertises 24-bit depth; hasColors(count) is true
        // for any request within that range.
        "getColorDepth" => Ok(Value::Float(24.0)),
        "hasColors" => Ok(Value::Bool(true)),
        _ => Err(crate::host::type_error(&format!("{method} is not a function"))),
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

/// The `fd` numeric property of a stream stand-in (default stdout).
fn stream_fd(recv: &Value) -> f64 {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("fd").map(|v| h.to_number(v)).unwrap_or(1.0),
        _ => 1.0,
    })
}

/// Write raw bytes to the process's stdout/stderr (chosen by fd).
fn write_fd(fd: f64, bytes: &[u8]) {
    use std::io::Write as _;
    if fd == 2.0 {
        let _ = std::io::stderr().write_all(bytes);
        let _ = std::io::stderr().flush();
    } else {
        let _ = std::io::stdout().write_all(bytes);
        let _ = std::io::stdout().flush();
    }
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
            ("involuntaryContextSwitches", ru.as_ref().map(|r| r.ru_nivcsw)),
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
            Some(JsObj::Object(p)) => {
                Some(p.iter().map(|(k, v)| format!("{k}={}", h.str_of(v))).collect::<Vec<_>>())
            }
            _ => None,
        });
        from_arg.unwrap_or_else(|| std::env::vars().map(|(k, v)| format!("{k}={v}")).collect())
    };

    let to_c = |s: String| CString::new(s).map_err(|_| crate::host::type_error("process.execve: NUL in argument"));
    let argv_c: Vec<CString> = argv_strs.into_iter().map(to_c).collect::<Result<_, _>>()?;
    let env_c: Vec<CString> = env_strs.into_iter().map(to_c).collect::<Result<_, _>>()?;

    let mut argv_p: Vec<*const libc::c_char> = argv_c.iter().map(|c| c.as_ptr()).collect();
    argv_p.push(std::ptr::null());
    let mut envp_p: Vec<*const libc::c_char> = env_c.iter().map(|c| c.as_ptr()).collect();
    envp_p.push(std::ptr::null());

    // SAFETY: argv/envp are NUL-terminated arrays of valid C strings kept alive
    // above; on success execve never returns.
    unsafe { libc::execve(prog.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr()); }
    Err(crate::host::type_error(&format!(
        "process.execve failed: {}",
        std::io::Error::last_os_error()
    )))
}

/// `process.loadEnvFile([path])` — parse a `.env` file into `process.env`
/// (persisted through the real environment so a later `process.env` read sees it).
fn load_env_file(path: &str) -> Result<Value, String> {
    let path = if path.is_empty() { ".env" } else { path };
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Error: ENOENT: {e}, open '{path}'"))?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else { continue };
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
