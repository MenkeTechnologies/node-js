//! Node `tty` module.
//!
//! `tty.isatty(fd)` queries whether a file descriptor is a terminal (via
//! `libc::isatty`). `tty.ReadStream`/`tty.WriteStream` are exposed as native
//! classes: a `WriteStream` carries `fd`/`isTTY`/`columns`/`rows` and the
//! cursor/erase methods (`cursorTo`/`moveCursor`/`clearLine`/`clearScreenDown`/
//! `getWindowSize`/`getColorDepth`/`hasColors`), all routed — like
//! `process.stdout` — through `process::stream_instance_call`; a `ReadStream`
//! carries `fd`/`isTTY`/`isRaw` with a best-effort `setRawMode`.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &["isatty"];

/// Instance methods of a `tty.ReadStream`.
pub const READ_STREAM_METHODS: &[&str] = &[
    "setRawMode",
    "on",
    "once",
    "removeListener",
    "pause",
    "resume",
    "setEncoding",
    "ref",
    "unref",
    "destroy",
    "read",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "isatty" => {
            let fd = super::arg_num(args, 0);
            let is = if fd.is_finite() {
                // SAFETY: isatty is a pure query on the given fd number.
                unsafe { libc::isatty(fd as libc::c_int) == 1 }
            } else {
                false
            };
            Ok(Value::Bool(is))
        }
        _ => return None,
    })
}

/// `require('tty').ReadStream` / `.WriteStream` — expose the classes as
/// constructible builtins (the parent routes `new`/instance calls by tag).
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "ReadStream" | "WriteStream" => Some(with_host(|h| h.alloc(JsObj::Builtin(name.into())))),
        _ => None,
    }
}

/// `new tty.ReadStream(fd)` / `new tty.WriteStream(fd)`.
pub fn construct(name: &str, args: &[Value]) -> Value {
    let fd = {
        let n = super::arg_num(args, 0);
        if n.is_finite() {
            n as i32
        } else if name == "WriteStream" {
            1
        } else {
            0
        }
    };
    match name {
        "ReadStream" => read_stream(fd),
        _ => write_stream(fd),
    }
}

/// Build a `tty.WriteStream` object (same shape as `process.stdout`).
pub fn write_stream(fd: i32) -> Value {
    // SAFETY: isatty is a pure query on the fd number.
    let is_tty = unsafe { libc::isatty(fd) == 1 };
    let size = if is_tty { window_size(fd) } else { None };
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("WriteStream"));
        m.insert("fd".into(), Value::Float(fd as f64));
        m.insert("isTTY".into(), Value::Bool(is_tty));
        m.insert("writable".into(), Value::Bool(true));
        if let Some((cols, rows)) = size {
            m.insert("columns".into(), Value::Float(cols as f64));
            m.insert("rows".into(), Value::Float(rows as f64));
        }
        h.new_object(m)
    })
}

/// Build a `tty.ReadStream` object.
pub fn read_stream(fd: i32) -> Value {
    // SAFETY: isatty is a pure query on the fd number.
    let is_tty = unsafe { libc::isatty(fd) == 1 };
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("ReadStream"));
        m.insert("fd".into(), Value::Float(fd as f64));
        m.insert("isTTY".into(), Value::Bool(is_tty));
        m.insert("isRaw".into(), Value::Bool(false));
        m.insert("readable".into(), Value::Bool(true));
        h.new_object(m)
    })
}

/// Dispatch a `tty.ReadStream` instance method. Terminal raw-mode toggling has no
/// termios substrate here, so `setRawMode` just records the flag; the rest are
/// chainable no-ops so `.on('data')`/`.pause()`/`.resume()` chains load.
pub fn instance_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "setRawMode" => {
            let mode = super::arg_num(args, 0) != 0.0;
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                    p.insert("isRaw".into(), Value::Bool(mode));
                }
            });
            Ok(recv.clone())
        }
        "read" => Ok(Value::Undef),
        "on" | "once" | "removeListener" | "pause" | "resume" | "setEncoding" | "ref" | "unref"
        | "destroy" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "{method} is not a function"
        ))),
    }
}

/// The terminal's `(columns, rows)` via `ioctl(TIOCGWINSZ)`; `None` when `fd` is
/// not a terminal or the ioctl fails.
pub fn window_size(fd: i32) -> Option<(u16, u16)> {
    // SAFETY: `ws` is zeroed then filled by the kernel; a failed ioctl returns -1.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ as _, &mut ws as *mut libc::winsize) == 0
            && ws.ws_col > 0
        {
            Some((ws.ws_col, ws.ws_row))
        } else {
            None
        }
    }
}
