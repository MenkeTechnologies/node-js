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
];

/// `os.EOL` constant.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "EOL" => Some(with_host(|h| h.new_str("\n"))),
        "devNull" => Some(with_host(|h| h.new_str("/dev/null"))),
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

pub fn call(method: &str, _args: &[Value]) -> Option<Result<Value, String>> {
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
        "homedir" => s(&dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
        "tmpdir" => s(std::env::temp_dir().to_string_lossy().trim_end_matches('/')),
        "endianness" => s(if cfg!(target_endian = "big") { "BE" } else { "LE" }),
        "totalmem" => Ok(Value::Float(0.0)),
        "freemem" => Ok(Value::Float(0.0)),
        "uptime" => Ok(Value::Float(0.0)),
        "cpus" => Ok(with_host(|h| h.new_array(Vec::new()))),
        "loadavg" => Ok(with_host(|h| h.new_array(vec![Value::Float(0.0); 3]))),
        "networkInterfaces" => Ok(with_host(|h| h.new_object(IndexMap::new()))),
        "userInfo" => Ok(user_info()),
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
        let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        let home = dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        m.insert("username".into(), h.new_str(user));
        m.insert("homedir".into(), h.new_str(home));
        m.insert("shell".into(), h.new_str(shell));
        m.insert("uid".into(), Value::Float(-1.0));
        m.insert("gid".into(), Value::Float(-1.0));
        h.new_object(m)
    })
}
