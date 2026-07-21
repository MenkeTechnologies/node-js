//! Node `os` module. Values that Node derives from the host (platform, arch,
//! hostname, home/tmp dirs, endianness, EOL) are returned faithfully; the
//! machine-specific numeric readings (`cpus`, `totalmem`, `freemem`, `loadavg`,
//! `uptime`) return best-effort placeholders (not fuzzed — they vary per host on
//! reference Node too).

use crate::host::with_host;
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
    "platform",
    "arch",
    "type",
    "release",
    "hostname",
    "homedir",
    "tmpdir",
    "endianness",
    "cpus",
    "totalmem",
    "freemem",
    "uptime",
    "loadavg",
    "userInfo",
    "networkInterfaces",
    "version",
    "machine",
    "availableParallelism",
    "getPriority",
    "setPriority",
];

/// `os.EOL` constant.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "EOL" => Some(with_host(|h| h.new_str("\n"))),
        "devNull" => Some(with_host(|h| h.new_str("/dev/null"))),
        // os.constants.signals (POSIX signal numbers) + priority levels.
        "constants" => Some(with_host(|h| {
            let mut signals = indexmap::IndexMap::new();
            for (k, v) in [
                ("SIGHUP", 1),
                ("SIGINT", 2),
                ("SIGQUIT", 3),
                ("SIGILL", 4),
                ("SIGTRAP", 5),
                ("SIGABRT", 6),
                ("SIGBUS", 10),
                ("SIGFPE", 8),
                ("SIGKILL", 9),
                ("SIGUSR1", 30),
                ("SIGSEGV", 11),
                ("SIGUSR2", 31),
                ("SIGPIPE", 13),
                ("SIGALRM", 14),
                ("SIGTERM", 15),
                ("SIGCHLD", 20),
                ("SIGCONT", 19),
                ("SIGSTOP", 17),
                ("SIGTSTP", 18),
                ("SIGWINCH", 28),
            ] {
                signals.insert(k.to_string(), Value::Float(v as f64));
            }
            let sig = h.new_object(signals);
            let mut priority = indexmap::IndexMap::new();
            for (k, v) in [
                ("PRIORITY_LOW", 19),
                ("PRIORITY_BELOW_NORMAL", 10),
                ("PRIORITY_NORMAL", 0),
                ("PRIORITY_ABOVE_NORMAL", -7),
                ("PRIORITY_HIGH", -14),
                ("PRIORITY_HIGHEST", -20),
            ] {
                priority.insert(k.to_string(), Value::Float(v as f64));
            }
            let prio = h.new_object(priority);
            let mut m = indexmap::IndexMap::new();
            m.insert("signals".to_string(), sig);
            m.insert("priority".to_string(), prio);
            h.new_object(m)
        })),
        _ => None,
    }
}

/// Node's `process.platform`/`os.platform()` string for the build target.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Node's `os.arch()`/`process.arch` string for the build target.
pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let s = |v: &str| Ok(with_host(|h| h.new_str(v)));
    Some(match method {
        "platform" => s(platform()),
        "arch" => s(arch()),
        "machine" => s(std::env::consts::ARCH),
        "type" => s(match std::env::consts::OS {
            "macos" => "Darwin",
            "linux" => "Linux",
            "windows" => "Windows_NT",
            other => other,
        }),
        "release" => s(""),
        "version" => s(""),
        "hostname" => s(&hostname()),
        "homedir" => s(&dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()),
        "tmpdir" => s(std::env::temp_dir().to_string_lossy().trim_end_matches('/')),
        "endianness" => s(if cfg!(target_endian = "big") {
            "BE"
        } else {
            "LE"
        }),
        "totalmem" => Ok(Value::Float(0.0)),
        "freemem" => Ok(Value::Float(0.0)),
        "uptime" => Ok(Value::Float(0.0)),
        "cpus" => Ok(with_host(|h| h.new_array(Vec::new()))),
        // Real 1/5/15-minute load averages via `getloadavg(3)`.
        "loadavg" => {
            let mut avg = [0f64; 3];
            // SAFETY: writes at most 3 doubles into a 3-element buffer.
            let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
            let items: Vec<Value> = if n == 3 {
                avg.iter().map(|v| Value::Float(*v)).collect()
            } else {
                vec![Value::Float(0.0); 3]
            };
            Ok(with_host(|h| h.new_array(items)))
        }
        "networkInterfaces" => Ok(with_host(|h| h.new_object(IndexMap::new()))),
        "userInfo" => Ok(user_info()),
        // Logical CPU count (Node uses libuv's available parallelism).
        "availableParallelism" => {
            let n = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            Ok(Value::Float(n as f64))
        }
        // `os.getPriority([pid])` — the nice value of `pid` (0 = current process).
        "getPriority" => {
            let pid = if args.is_empty() {
                0
            } else {
                super::arg_num(args, 0) as i32
            };
            // SAFETY: pure query; PRIO_PROCESS with a pid.
            let prio = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, pid as _) };
            Ok(Value::Float(prio as f64))
        }
        // `os.setPriority([pid, ]priority)` — best-effort (needs privilege to lower
        // the nice value); returns undefined.
        "setPriority" => {
            let (pid, prio) = if args.len() >= 2 {
                (
                    super::arg_num(args, 0) as i32,
                    super::arg_num(args, 1) as i32,
                )
            } else {
                (0, super::arg_num(args, 0) as i32)
            };
            // SAFETY: PRIO_PROCESS with a pid and nice value; failure returns -1.
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS as _, pid as _, prio as _);
            }
            Ok(Value::Undef)
        }
        _ => return None,
    })
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn user_info() -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        let home = dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        m.insert("username".into(), h.new_str(user));
        m.insert("homedir".into(), h.new_str(home));
        m.insert("shell".into(), h.new_str(shell));
        m.insert("uid".into(), Value::Float(-1.0));
        m.insert("gid".into(), Value::Float(-1.0));
        h.new_object(m)
    })
}
