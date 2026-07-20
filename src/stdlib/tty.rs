//! Node `tty` module — the subset packages read at load time.
//!
//! `tty.isatty(fd)` queries whether a file descriptor is a terminal (via
//! `libc::isatty`); the `ReadStream`/`WriteStream` classes are not modeled (no
//! package in the express tree constructs them at require-time).

use fusevm::Value;

pub const METHODS: &[&str] = &["isatty"];

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
