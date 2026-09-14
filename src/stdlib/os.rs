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
        // Every number comes from `libc`, so it is the one THIS platform uses:
        // the signal table was a hardcoded list of macOS values, which made
        // `SIGUSR1` 30 (Linux uses 10) and `SIGSTOP` 17 (Linux uses 19) on
        // every Linux host. `errno` and `dlopen` were missing entirely.
        "constants" => {
            // Each sub-table allocates, so it is built BEFORE the borrow below
            // rather than inside it.
            let sig = super::constants::object(&super::constants::signals());
            let prio = super::constants::object(&super::constants::priority());
            let errno = super::constants::object(&super::constants::errno());
            let dlopen = super::constants::object(&super::constants::dlopen());
            Some(with_host(|h| {
                let mut m = indexmap::IndexMap::new();
                m.insert("UV_UDP_REUSEADDR".to_string(), Value::Float(4.0));
                m.insert("dlopen".to_string(), dlopen);
                m.insert("errno".to_string(), errno);
                m.insert("signals".to_string(), sig);
                m.insert("priority".to_string(), prio);
                h.new_object(m)
            }))
        }
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
        // Physical memory, from `sysconf`. Answering 0 is not a neutral
        // placeholder: `os.totalmem()` is read to size caches and worker pools,
        // and zero bytes of RAM is a value no machine reports.
        "totalmem" => Ok(Value::Float(phys_bytes(libc::_SC_PHYS_PAGES))),
        "freemem" => Ok(Value::Float(free_bytes())),
        "uptime" => Ok(Value::Float(uptime_secs())),
        "cpus" => Ok(cpus()),
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
        "networkInterfaces" => Ok(network_interfaces()),
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

/// Bytes of physical memory behind a `sysconf` page count. Both platforms this
/// targets expose `_SC_PHYS_PAGES`; anything that does not answers 0, which is
/// what the whole family used to do unconditionally.
fn phys_bytes(name: libc::c_int) -> f64 {
    // SAFETY: `sysconf` reads a constant and returns a long; -1 signals absent.
    let pages = unsafe { libc::sysconf(name) };
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || page <= 0 {
        return 0.0;
    }
    pages as f64 * page as f64
}

/// Free physical memory. Linux has a `sysconf` for it; macOS does not, so the
/// page counts come from the Mach VM statistics the same way libuv reads them.
#[cfg(target_os = "linux")]
fn free_bytes() -> f64 {
    phys_bytes(libc::_SC_AVPHYS_PAGES)
}

#[cfg(target_os = "macos")]
fn free_bytes() -> f64 {
    // `vm.page_free_count` rather than Mach's `host_statistics64`:
    // `libc::mach_host_self` is deprecated in favour of a separate crate, and
    // this reads the same counter through the `sysctl` path already used here.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        return 0.0;
    }
    // libuv counts the free list PLUS the speculative pages, which the kernel
    // hands back on demand — free alone reports roughly a third of what node
    // does on the same machine.
    let free = sysctl_u32(c"vm.page_free_count").unwrap_or(0) as f64;
    let spec = sysctl_u32(c"vm.page_speculative_count").unwrap_or(0) as f64;
    (free + spec) * page as f64
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn free_bytes() -> f64 {
    0.0
}

/// Seconds since boot.
#[cfg(target_os = "linux")]
fn uptime_secs() -> f64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0)
}

#[cfg(target_os = "macos")]
fn uptime_secs() -> f64 {
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::timeval>();
    // SAFETY: `sysctl` writes at most `len` bytes into `tv`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            &mut tv as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || tv.tv_sec == 0 {
        return 0.0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    (now - tv.tv_sec as f64).max(0.0).floor()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn uptime_secs() -> f64 {
    0.0
}

/// `os.cpus()` — one entry per logical core, each with the CPU's model string,
/// its nominal speed in MHz, and the scheduler's time counters.
///
/// It answered an EMPTY array, which is the shape `os.cpus().length` is read
/// for: sizing a worker pool off zero cores, or `|| 1` masking it.
fn cpus() -> Value {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let model = cpu_model();
    let speed = cpu_speed_mhz();
    with_host(|h| {
        let items: Vec<Value> = (0..n)
            .map(|_| {
                let mut times = IndexMap::new();
                // The per-core counters need Mach's `host_processor_info` on
                // macOS and `/proc/stat` on Linux; libuv reads them per core and
                // this does not, so they are reported as zero rather than
                // invented. The COUNT, model and speed are real.
                for k in ["user", "nice", "sys", "idle", "irq"] {
                    times.insert(k.to_string(), Value::Float(0.0));
                }
                let times = h.new_object(times);
                let mut m = IndexMap::new();
                m.insert("model".into(), h.new_str(model.clone()));
                m.insert("speed".into(), Value::Float(speed));
                m.insert("times".into(), times);
                h.new_object(m)
            })
            .collect();
        h.new_array(items)
    })
}

#[cfg(target_os = "macos")]
fn cpu_model() -> String {
    sysctl_string(c"machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into())
}

#[cfg(target_os = "linux")]
fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name") || l.starts_with("Model"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn cpu_model() -> String {
    "unknown".into()
}

#[cfg(target_os = "macos")]
fn cpu_speed_mhz() -> f64 {
    // Apple Silicon does not expose `hw.cpufrequency`; libuv reports 0 there
    // too rather than guessing.
    sysctl_u64(c"hw.cpufrequency").map_or(0.0, |hz| (hz / 1_000_000) as f64)
}

#[cfg(target_os = "linux")]
fn cpu_speed_mhz() -> f64 {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("cpu MHz"))
                .and_then(|l| l.split_once(':'))
                .and_then(|(_, v)| v.trim().parse::<f64>().ok())
        })
        .map(|f| f.round())
        .unwrap_or(0.0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn cpu_speed_mhz() -> f64 {
    0.0
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut len: usize = 0;
    // SAFETY: a null buffer asks only for the length.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || len == 0
    {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: writes at most `len` bytes into a buffer of that size.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buf.pop();
    String::from_utf8(buf).ok()
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &std::ffi::CStr) -> Option<u32> {
    let mut out: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    // SAFETY: writes at most 4 bytes into `out`.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut out as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(out)
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &std::ffi::CStr) -> Option<u64> {
    let mut out: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: writes at most 8 bytes into `out`.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut out as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(out)
}

/// `os.networkInterfaces()` — the addresses `getifaddrs(3)` reports, grouped by
/// interface name, in node's shape. It answered an EMPTY object, which reads as
/// a machine with no network at all.
///
/// Only IPv4 and IPv6 entries become addresses; a link-layer entry supplies the
/// interface's MAC, which node attaches to every address of that interface.
fn network_interfaces() -> Value {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` allocates the list; `freeifaddrs` releases it below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return with_host(|h| h.new_object(IndexMap::new()));
    }
    let mut macs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut addrs: Vec<(String, IfAddr)> = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: the list is well-formed until `freeifaddrs`.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: `ifa_name` is a NUL-terminated interface name.
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        if let Some(mac) = link_mac(ifa) {
            macs.insert(name.clone(), mac);
            continue;
        }
        if let Some(a) = ip_addr(ifa) {
            addrs.push((name, a));
        }
    }
    // SAFETY: `head` came from `getifaddrs` and is freed exactly once.
    unsafe { libc::freeifaddrs(head) };
    with_host(|h| {
        let mut grouped: IndexMap<String, Vec<Value>> = IndexMap::new();
        for (name, a) in addrs {
            let mac = macs
                .get(&name)
                .cloned()
                .unwrap_or_else(|| "00:00:00:00:00:00".into());
            let mut m = IndexMap::new();
            m.insert("address".into(), h.new_str(a.address.clone()));
            m.insert("netmask".into(), h.new_str(a.netmask.clone()));
            m.insert("family".into(), h.new_str(a.family.to_string()));
            m.insert("mac".into(), h.new_str(mac));
            m.insert("internal".into(), Value::Bool(a.internal));
            m.insert(
                "cidr".into(),
                h.new_str(format!("{}/{}", a.address, a.prefix)),
            );
            grouped.entry(name).or_default().push(h.new_object(m));
        }
        let mut out = IndexMap::new();
        for (name, list) in grouped {
            let arr = h.new_array(list);
            out.insert(name, arr);
        }
        h.new_object(out)
    })
}

struct IfAddr {
    address: String,
    netmask: String,
    family: &'static str,
    internal: bool,
    prefix: u32,
}

/// The MAC of a link-layer entry, or `None` for an address entry.
#[cfg(target_os = "macos")]
fn link_mac(ifa: &libc::ifaddrs) -> Option<String> {
    if ifa.ifa_addr.is_null() {
        return None;
    }
    // SAFETY: `sa_family` is the first field of every `sockaddr`.
    if unsafe { (*ifa.ifa_addr).sa_family } as i32 != libc::AF_LINK {
        return None;
    }
    // SAFETY: an `AF_LINK` address is a `sockaddr_dl`.
    let dl = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_dl) };
    let start = dl.sdl_nlen as usize;
    let len = dl.sdl_alen as usize;
    if len != 6 || start + len > dl.sdl_data.len() {
        return None;
    }
    let b: Vec<String> = dl.sdl_data[start..start + len]
        .iter()
        .map(|c| format!("{:02x}", *c as u8))
        .collect();
    Some(b.join(":"))
}

#[cfg(target_os = "linux")]
fn link_mac(ifa: &libc::ifaddrs) -> Option<String> {
    if ifa.ifa_addr.is_null() {
        return None;
    }
    // SAFETY: `sa_family` is the first field of every `sockaddr`.
    if unsafe { (*ifa.ifa_addr).sa_family } as i32 != libc::AF_PACKET {
        return None;
    }
    // SAFETY: an `AF_PACKET` address is a `sockaddr_ll`.
    let ll = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_ll) };
    let len = ll.sll_halen as usize;
    if len != 6 {
        return None;
    }
    let b: Vec<String> = ll.sll_addr[..len]
        .iter()
        .map(|c| format!("{c:02x}"))
        .collect();
    Some(b.join(":"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn link_mac(_ifa: &libc::ifaddrs) -> Option<String> {
    None
}

/// The IPv4/IPv6 address of an entry, with its netmask and prefix length.
fn ip_addr(ifa: &libc::ifaddrs) -> Option<IfAddr> {
    if ifa.ifa_addr.is_null() {
        return None;
    }
    // SAFETY: `sa_family` is the first field of every `sockaddr`.
    let fam = unsafe { (*ifa.ifa_addr).sa_family } as i32;
    let internal = ifa.ifa_flags & libc::IFF_LOOPBACK as u32 != 0;
    if fam == libc::AF_INET {
        // SAFETY: an `AF_INET` address is a `sockaddr_in`.
        let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
        let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
        let mask = if ifa.ifa_netmask.is_null() {
            std::net::Ipv4Addr::UNSPECIFIED
        } else {
            // SAFETY: the netmask of an `AF_INET` entry is a `sockaddr_in`.
            let m = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in) };
            std::net::Ipv4Addr::from(u32::from_be(m.sin_addr.s_addr))
        };
        return Some(IfAddr {
            address: ip.to_string(),
            netmask: mask.to_string(),
            family: "IPv4",
            internal,
            prefix: u32::from(mask).count_ones(),
        });
    }
    if fam == libc::AF_INET6 {
        // SAFETY: an `AF_INET6` address is a `sockaddr_in6`.
        let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
        let ip = std::net::Ipv6Addr::from(sin.sin6_addr.s6_addr);
        let mask = if ifa.ifa_netmask.is_null() {
            std::net::Ipv6Addr::UNSPECIFIED
        } else {
            // SAFETY: the netmask of an `AF_INET6` entry is a `sockaddr_in6`.
            let m = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in6) };
            std::net::Ipv6Addr::from(m.sin6_addr.s6_addr)
        };
        let prefix: u32 = mask.octets().iter().map(|b| b.count_ones()).sum();
        return Some(IfAddr {
            address: ip.to_string(),
            netmask: mask.to_string(),
            family: "IPv6",
            internal,
            prefix,
        });
    }
    None
}
