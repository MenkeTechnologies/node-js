//! Node `fs` module: synchronous file operations, the async callback forms, and
//! the file-descriptor / directory / stream / watcher surfaces.
//!
//! Path operations map straight onto `std::fs`; descriptor operations
//! (`open`/`read`/`write`/`fstat`/…) keep a thread-local table of open
//! `std::fs::File`s keyed by a synthetic integer fd (starting at 3, after the
//! stdio fds). Ownership / timestamp / statvfs operations that `std::fs` does not
//! cover use `libc` directly (all targets are unix). Async callback variants run
//! the same synchronous work then schedule the callback as a microtask, matching
//! Node's ordering for simple scripts.

use super::{arg_num, arg_str, native_tag};
use crate::host::{invoke, with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

pub const METHODS: &[&str] = &[
    // path — sync
    "readFileSync",
    "writeFileSync",
    "appendFileSync",
    "existsSync",
    "readdirSync",
    "mkdirSync",
    "rmdirSync",
    "unlinkSync",
    "rmSync",
    "statSync",
    "lstatSync",
    "statfsSync",
    "accessSync",
    "chmodSync",
    "chownSync",
    "lchownSync",
    "copyFileSync",
    "cpSync",
    "linkSync",
    "symlinkSync",
    "readlinkSync",
    "realpathSync",
    "renameSync",
    "truncateSync",
    "utimesSync",
    "lutimesSync",
    "mkdtempSync",
    "opendirSync",
    "globSync",
    // fd — sync
    "openSync",
    "closeSync",
    "readSync",
    "writeSync",
    "readvSync",
    "writevSync",
    "fstatSync",
    "fchmodSync",
    "fchownSync",
    "ftruncateSync",
    "futimesSync",
    "fsyncSync",
    "fdatasyncSync",
    // path — async (callback)
    "readFile",
    "writeFile",
    "appendFile",
    "readdir",
    "mkdir",
    "rmdir",
    "rm",
    "unlink",
    "stat",
    "lstat",
    "statfs",
    "access",
    "chmod",
    "chown",
    "lchown",
    "copyFile",
    "cp",
    "link",
    "symlink",
    "readlink",
    "realpath",
    "rename",
    "truncate",
    "utimes",
    "lutimes",
    "mkdtemp",
    "opendir",
    "glob",
    "exists",
    // fd — async (callback)
    "open",
    "close",
    "read",
    "write",
    "readv",
    "writev",
    "fstat",
    "fchmod",
    "fchown",
    "ftruncate",
    "futimes",
    "fsync",
    "fdatasync",
    // watchers + streams
    "watchFile",
    "unwatchFile",
    "createReadStream",
    "createWriteStream",
];

// ── file-descriptor table ────────────────────────────────────────────────────

thread_local! {
    static FD_TABLE: RefCell<HashMap<i32, File>> = RefCell::new(HashMap::new());
    static NEXT_FD: RefCell<i32> = const { RefCell::new(3) };
    static WATCHERS: RefCell<Vec<WatchEntry>> = const { RefCell::new(Vec::new()) };
    static NEXT_WATCH_ID: RefCell<u64> = const { RefCell::new(1) };
}

struct WatchEntry {
    // Reserved for a future StatWatcher handle; assigned but not yet read.
    #[allow(dead_code)]
    id: u64,
    path: String,
    listener: Value,
    stop: Arc<AtomicBool>,
}

fn register_fd(file: File) -> i32 {
    NEXT_FD.with(|n| {
        let fd = *n.borrow();
        *n.borrow_mut() = fd + 1;
        FD_TABLE.with(|t| t.borrow_mut().insert(fd, file));
        fd
    })
}

fn with_file<R>(fd: i32, f: impl FnOnce(&File) -> R) -> Option<R> {
    FD_TABLE.with(|t| t.borrow().get(&fd).map(f))
}

/// The file descriptor an fs call names when its first argument is a number
/// rather than a path.
fn fd_arg(v: Option<&Value>) -> Option<i32> {
    match v? {
        Value::Int(n) => Some(*n as i32),
        Value::Float(n) if n.fract() == 0.0 => Some(*n as i32),
        _ => None,
    }
}

/// Everything left to read on `fd`: standard input for `0`, otherwise a
/// descriptor `openSync` handed out. `fstat` is the call node fails on for an
/// unknown descriptor.
fn read_fd_to_end(fd: i32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let r = if fd == 0 {
        // Through Rust's shared stdin buffer, so a read here and a
        // `readline` read of the same input never skip each other's bytes.
        std::io::stdin().lock().read_to_end(&mut out)
    } else {
        match with_file(fd, |file| {
            let mut fr: &File = file;
            fr.read_to_end(&mut out)
        }) {
            Some(r) => r,
            None => return Err("Error: EBADF: bad file descriptor, fstat".to_string()),
        }
    };
    r.map(|_| out).map_err(|e| err_str("read", "", &e))
}

fn close_fd(fd: i32) -> bool {
    FD_TABLE.with(|t| t.borrow_mut().remove(&fd).is_some())
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let op = method.strip_suffix("Sync").unwrap_or(method);
    // `existsSync` reports an unusable path as absent rather than throwing.
    if op == "exists" && validated_paths("exists", args).is_err() {
        return Some(match method {
            "existsSync" => Ok(Value::Bool(false)),
            _ => exists_async(&[Value::Undef, args.get(1).cloned().unwrap_or(Value::Undef)]),
        });
    }
    let normalized = match validated_paths(op, args) {
        Ok(Some(a)) => a,
        Ok(None) => args.to_vec(),
        Err(e) => return Some(Err(e)),
    };
    let args = normalized.as_slice();
    if let Err(e) = validate_call(op, args) {
        return Some(Err(e));
    }
    Some(match method {
        // ── path, sync ──
        "readFileSync" => read_file_sync(args),
        "writeFileSync" => write_file_impl(args),
        "appendFileSync" => append_file_impl(args),
        "existsSync" => Ok(Value::Bool(Path::new(&arg_str(args, 0)).exists())),
        "readdirSync" => readdir_impl(args),
        "mkdirSync" => mkdir_impl(args),
        "rmdirSync" | "unlinkSync" | "rmSync" => rm_impl(method, args),
        "statSync" => stat_impl("statSync", args, true),
        "lstatSync" => stat_impl("lstatSync", args, false),
        "statfsSync" => statfs_impl(args),
        "accessSync" => access_impl(args),
        "chmodSync" => chmod_impl(args),
        "chownSync" => chown_impl(args, true),
        "lchownSync" => chown_impl(args, false),
        "copyFileSync" => copy_file_impl(args),
        "cpSync" => cp_impl(args),
        "linkSync" => link_impl(args),
        "symlinkSync" => symlink_impl(args),
        "readlinkSync" => readlink_impl(args),
        "realpathSync" => realpath_impl(args),
        "renameSync" => rename_impl(args),
        "truncateSync" => truncate_impl(args),
        "utimesSync" => utimes_impl(args, true),
        "lutimesSync" => utimes_impl(args, false),
        "mkdtempSync" => mkdtemp_impl(args),
        "opendirSync" => opendir_impl(args),
        "globSync" => glob_impl(args),
        // ── fd, sync ──
        "openSync" => open_impl(args),
        "closeSync" => close_impl(args),
        "readSync" => read_impl(args).map(|n| Value::Float(n as f64)),
        "writeSync" => write_impl(args).map(|n| Value::Float(n as f64)),
        "readvSync" => readv_impl(args).map(|n| Value::Float(n as f64)),
        "writevSync" => writev_impl(args).map(|n| Value::Float(n as f64)),
        "fstatSync" => fstat_impl(args),
        "fchmodSync" => fchmod_impl(args),
        "fchownSync" => fchown_impl(args),
        "ftruncateSync" => ftruncate_impl(args),
        "futimesSync" => futimes_impl(args),
        "fsyncSync" => fsync_impl(args, false),
        "fdatasyncSync" => fsync_impl(args, true),
        // ── path, async ──
        "readFile" => return Some(read_file_async(args)),
        "writeFile" => async_cb(args, write_file_impl(args)),
        "appendFile" => async_cb(args, append_file_impl(args)),
        "readdir" => async_cb(args, readdir_impl(args)),
        "mkdir" => async_cb(args, mkdir_impl(args)),
        "rmdir" => async_cb(args, rm_impl("rmdir", args)),
        "rm" => async_cb(args, rm_impl("rm", args)),
        "unlink" => async_cb(args, rm_impl("unlink", args)),
        "stat" => async_cb(args, stat_impl("stat", args, true)),
        "lstat" => async_cb(args, stat_impl("lstat", args, false)),
        "statfs" => async_cb(args, statfs_impl(args)),
        "access" => async_cb(args, access_impl(args)),
        "chmod" => async_cb(args, chmod_impl(args)),
        "chown" => async_cb(args, chown_impl(args, true)),
        "lchown" => async_cb(args, chown_impl(args, false)),
        "copyFile" => async_cb(args, copy_file_impl(args)),
        "cp" => async_cb(args, cp_impl(args)),
        "link" => async_cb(args, link_impl(args)),
        "symlink" => async_cb(args, symlink_impl(args)),
        "readlink" => async_cb(args, readlink_impl(args)),
        "realpath" => async_cb(args, realpath_impl(args)),
        "rename" => async_cb(args, rename_impl(args)),
        "truncate" => async_cb(args, truncate_impl(args)),
        "utimes" => async_cb(args, utimes_impl(args, true)),
        "lutimes" => async_cb(args, utimes_impl(args, false)),
        "mkdtemp" => async_cb(args, mkdtemp_impl(args)),
        "opendir" => async_cb(args, opendir_impl(args)),
        "glob" => async_cb(args, glob_impl(args)),
        "exists" => exists_async(args),
        // ── fd, async ──
        "open" => async_cb(args, open_impl(args)),
        "close" => async_cb(args, close_impl(args)),
        "read" => read_async(args),
        "write" => return Some(read_write_async(args, write_impl(args))),
        "readv" => async_cb(args, readv_impl(args).map(|n| Value::Float(n as f64))),
        "writev" => async_cb(args, writev_impl(args).map(|n| Value::Float(n as f64))),
        "fstat" => async_cb(args, fstat_impl(args)),
        "fchmod" => async_cb(args, fchmod_impl(args)),
        "fchown" => async_cb(args, fchown_impl(args)),
        "ftruncate" => async_cb(args, ftruncate_impl(args)),
        "futimes" => async_cb(args, futimes_impl(args)),
        "fsync" => async_cb(args, fsync_impl(args, false)),
        "fdatasync" => async_cb(args, fsync_impl(args, true)),
        // ── watchers + streams ──
        "watchFile" => watch_file(args),
        "unwatchFile" => unwatch_file(args),
        "createReadStream" => create_read_stream(args),
        "createWriteStream" => create_write_stream(args),
        _ => return None,
    })
}

// ── async callback plumbing ──────────────────────────────────────────────────

/// Schedule the trailing callback as a microtask: `cb(null, value)` on success,
/// `cb(err)` (an error string, matching the existing `readFile`/`writeFile`
/// callbacks) on failure. Always returns `undefined` (the method's own result).
fn async_cb(args: &[Value], result: Result<Value, String>) -> Result<Value, String> {
    let Some(cb) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    match result {
        Ok(v) => with_host(|h| {
            let n = h.null();
            h.queue_io_callback(cb, vec![n, v]);
        }),
        // An error-first callback is handed an ERROR OBJECT, not the message:
        // `fs.stat(missing, (err) => …)` gives an `err` carrying `code`,
        // `syscall`, `path` and `errno`, and `err.code === 'ENOENT'` is the most
        // common check in node code. A bare string made every one of those
        // `undefined` — and `err.message` too, so even printing it said
        // `undefined`. The promise forms already built the object.
        Err(e) => with_host(|h| {
            let ev = crate::builtins::synth_error(h, &e);
            h.queue_io_callback(cb, vec![ev]);
        }),
    }
    Ok(Value::Undef)
}

/// `read`/`write` callbacks carry a third argument: `cb(err, count, buffer)`.
fn read_write_async(args: &[Value], result: Result<usize, String>) -> Result<Value, String> {
    let Some(cb) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    let buffer = args.get(1).cloned().unwrap_or(Value::Undef);
    match result {
        Ok(n) => with_host(|h| {
            let nul = h.null();
            h.queue_io_callback(cb, vec![nul, Value::Float(n as f64), buffer]);
        }),
        Err(e) => with_host(|h| {
            let ev = crate::builtins::synth_error(h, &e);
            h.queue_io_callback(cb, vec![ev]);
        }),
    }
    Ok(Value::Undef)
}

fn is_fn(v: &Value) -> bool {
    with_host(|h| crate::host::is_callable(h, v))
}

// ── read/write/append ────────────────────────────────────────────────────────

fn read_file_sync(args: &[Value]) -> Result<Value, String> {
    let enc = encoding_arg(args, 1);
    // A NUMBER is a file descriptor, read from its current position to the
    // end: `readFileSync(0, 'utf8')` is how a script slurps its stdin. The
    // number was stringified and opened as a path, so that was `ENOENT ... '0'`.
    if let Some(fd) = fd_arg(args.first()) {
        let bytes = read_fd_to_end(fd)?;
        return Ok(match enc {
            Some(e) => with_host(|h| h.new_str(super::buffer::encode_bytes(&bytes, &e))),
            None => super::buffer::from_bytes(&bytes),
        });
    }
    let path = arg_str(args, 0);

    match std::fs::read(&path) {
        // The encoding names the REPRESENTATION to return, not just "decode as
        // text": `readFileSync(p, 'hex')` gives the hex digits of the file, and
        // `'base64'` its base64. Every encoding used to come back as the raw
        // UTF-8 text.
        Ok(bytes) => Ok(match enc {
            Some(e) => with_host(|h| h.new_str(super::buffer::encode_bytes(&bytes, &e))),
            None => super::buffer::from_bytes(&bytes),
        }),
        // A DIRECTORY opens fine and fails at the `read`, which is the syscall
        // node names — and it names no path there, because the descriptor no
        // longer carries one. This reported `open '<dir>'`, an operation that
        // had in fact succeeded.
        // The raw errno rather than `ErrorKind::IsADirectory`, which is stable
        // only since 1.83 and this crate's MSRV is 1.80.
        Err(e) if e.raw_os_error() == Some(libc::EISDIR) => Err(err_str("read", "", &e)),
        Err(e) => Err(err_str("readFileSync", &path, &e)),
    }
}

/// The bytes a `writeFileSync`/`appendFileSync` should put on disk.
///
/// When an encoding is given, the DATA STRING is in that encoding and has to be
/// decoded first — `writeFileSync(p, '68656c6c6f', 'hex')` writes `hello`, five
/// bytes, not the ten digits. The encoding argument was ignored, so the digits
/// themselves were written. A Buffer argument carries its own bytes and is
/// unaffected.
fn encoded_bytes(args: &[Value]) -> Vec<u8> {
    let v = args.get(1).cloned().unwrap_or(Value::Undef);
    // Any byte VIEW writes its bytes; only a string goes through the encoding.
    // The Buffer case alone was handled, so `writeFileSync(p, new Uint8Array(…))`
    // wrote the text `[object Object]`.
    if let Some(bytes) = super::buffer::view_bytes(&v) {
        return bytes;
    }
    let text = with_host(|h| h.str_of(&v));
    match encoding_arg(args, 2) {
        Some(e) => super::buffer::decode_str(&text, &e),
        None => text.into_bytes(),
    }
}

fn write_file_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let data = encoded_bytes(args);
    match std::fs::write(&path, data) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("writeFile", &path, &e)),
    }
}

fn append_file_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let data = encoded_bytes(args);
    let r = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(&data));
    match r {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("appendFile", &path, &e)),
    }
}

fn read_file_async(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let Some(cb) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    let enc = if args.len() >= 3 {
        encoding_arg(args, 1)
    } else {
        None
    };
    let (err, data) = match std::fs::read(&path) {
        Ok(bytes) => (
            with_host(|h| h.null()),
            // Same representation rule as the sync read: the encoding names
            // what to return, so `readFile(p, 'hex', cb)` hands the callback
            // hex digits.
            match enc {
                Some(e) => with_host(|h| h.new_str(super::buffer::encode_bytes(&bytes, &e))),
                None => super::buffer::from_bytes(&bytes),
            },
        ),
        Err(e) => (
            // An ERROR OBJECT, as every error-first callback receives — see
            // `async_cb`; this path built its own and handed over a string.
            with_host(|h| {
                // The ASYNC form names the path where the sync one does not:
                // node's `fs.readFile(dir, cb)` reports `read '<dir>'` and
                // `fs.readFileSync(dir)` reports a bare `read`. Measured, not
                // assumed — the two really do differ.
                let msg = if e.raw_os_error() == Some(libc::EISDIR) {
                    err_str("read", &path, &e)
                } else {
                    err_str("readFile", &path, &e)
                };
                crate::builtins::synth_error(h, &msg)
            }),
            Value::Undef,
        ),
    };
    with_host(|h| h.queue_io_callback(cb, vec![err, data]));
    Ok(Value::Undef)
}

fn exists_async(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let Some(cb) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    let ex = Path::new(&path).exists();
    with_host(|h| h.queue_io_callback(cb, vec![Value::Bool(ex)]));
    Ok(Value::Undef)
}

// ── directories ──────────────────────────────────────────────────────────────

fn mkdir_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let recursive = opt_flag(args, "recursive");
    let r = if recursive {
        std::fs::create_dir_all(&path)
    } else {
        std::fs::create_dir(&path)
    };
    match r {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("mkdir", &path, &e)),
    }
}

fn rm_impl(op: &str, args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let p = Path::new(&path);
    let force = opt_flag(args, "force");
    let r = if p.is_dir() {
        if opt_flag(args, "recursive") {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_dir(p)
        }
    } else {
        std::fs::remove_file(p)
    };
    match r {
        Ok(_) => Ok(Value::Undef),
        Err(e) if force && e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Undef),
        Err(e) => Err(err_str(op, &path, &e)),
    }
}

fn readdir_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let file_types = opt_flag(args, "withFileTypes");
    let recursive = opt_flag(args, "recursive");
    let mut names: Vec<(String, String, std::fs::FileType)> = Vec::new();
    collect_dir(Path::new(&path), &path, "", recursive, &mut names)
        .map_err(|e| err_str("readdir", &path, &e))?;
    names.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(with_host(|h| {
        let items: Vec<Value> = names
            .into_iter()
            .map(|(rel, parent, ft)| {
                if file_types {
                    let base = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                    build_dirent(h, base, &parent, ft)
                } else {
                    h.new_str(rel)
                }
            })
            .collect();
        h.new_array(items)
    }))
}

/// Collect directory entries. `rel_prefix` is the path (relative to the root)
/// under which `dir`'s children are listed; `parent` is the absolute-ish parent
/// path recorded on each `Dirent`.
fn collect_dir(
    dir: &Path,
    parent: &str,
    rel_prefix: &str,
    recursive: bool,
    out: &mut Vec<(String, String, std::fs::FileType)>,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let ft = e.file_type()?;
        out.push((rel.clone(), parent.to_string(), ft));
        if recursive && ft.is_dir() {
            let sub = e.path();
            let sub_parent = sub.to_string_lossy().into_owned();
            collect_dir(&sub, &sub_parent, &rel, recursive, out)?;
        }
    }
    Ok(())
}

fn opendir_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let rd = std::fs::read_dir(&path).map_err(|e| err_str("opendir", &path, &e))?;
    let mut entries: Vec<(String, std::fs::FileType)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_type()
                .ok()
                .map(|ft| (e.file_name().to_string_lossy().into_owned(), ft))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(with_host(|h| {
        let dirents: Vec<Value> = entries
            .into_iter()
            .map(|(name, ft)| build_dirent(h, name, &path, ft))
            .collect();
        let arr = h.new_array(dirents);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("Dir"));
        m.insert("path".into(), h.new_str(path.clone()));
        m.insert("@@entries".into(), arr);
        m.insert("@@pos".into(), Value::Float(0.0));
        h.new_object(m)
    }))
}

// ── ownership / permissions / timestamps ─────────────────────────────────────

fn access_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::metadata(&path) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("access", &path, &e)),
    }
}

fn chmod_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let mode = parse_file_mode(args.get(1).unwrap_or(&Value::Undef), "mode", None)?;
    match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("chmod", &path, &e)),
    }
}

fn chown_impl(args: &[Value], follow: bool) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let uid = arg_num(args, 1) as libc::uid_t;
    let gid = arg_num(args, 2) as libc::gid_t;
    let c = cpath(&path, "chown")?;
    let rc = unsafe {
        if follow {
            libc::chown(c.as_ptr(), uid, gid)
        } else {
            libc::lchown(c.as_ptr(), uid, gid)
        }
    };
    ok_or_errno(rc, "chown", &path)
}

fn utimes_impl(args: &[Value], follow: bool) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let times = [
        to_timeval(time_secs(args, 1)),
        to_timeval(time_secs(args, 2)),
    ];
    let c = cpath(&path, "utimes")?;
    let rc = unsafe {
        if follow {
            libc::utimes(c.as_ptr(), times.as_ptr())
        } else {
            libc::lutimes(c.as_ptr(), times.as_ptr())
        }
    };
    ok_or_errno(rc, "utimes", &path)
}

// ── copy / link / rename / truncate ──────────────────────────────────────────

const COPYFILE_EXCL: u32 = 1;

fn copy_file_impl(args: &[Value]) -> Result<Value, String> {
    let src = arg_str(args, 0);
    let dest = arg_str(args, 1);
    let mode = arg_num(args, 2);
    if !mode.is_nan() && (mode as u32) & COPYFILE_EXCL != 0 && Path::new(&dest).exists() {
        return Err(format!(
            "Error: EEXIST: file already exists, copyfile '{src}' -> '{dest}'"
        ));
    }
    match std::fs::copy(&src, &dest) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str2("copyFile", &src, &dest, &e)),
    }
}

fn cp_impl(args: &[Value]) -> Result<Value, String> {
    let src = arg_str(args, 0);
    let dest = arg_str(args, 1);
    let recursive = opt_flag(args, "recursive");
    let r = if recursive {
        cp_recursive(Path::new(&src), Path::new(&dest))
    } else {
        std::fs::copy(&src, &dest).map(|_| ())
    };
    match r {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("cp", &src, &e)),
    }
}

fn cp_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            cp_recursive(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::copy(src, dest).map(|_| ())
    }
}

fn link_impl(args: &[Value]) -> Result<Value, String> {
    let existing = arg_str(args, 0);
    let new = arg_str(args, 1);
    match std::fs::hard_link(&existing, &new) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("link", &existing, &e)),
    }
}

fn symlink_impl(args: &[Value]) -> Result<Value, String> {
    let target = arg_str(args, 0);
    let path = arg_str(args, 1);
    match std::os::unix::fs::symlink(&target, &path) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("symlink", &path, &e)),
    }
}

fn readlink_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::read_link(&path) {
        Ok(p) => Ok(with_host(|h| h.new_str(p.to_string_lossy().into_owned()))),
        Err(e) => Err(err_str("readlink", &path, &e)),
    }
}

fn realpath_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::canonicalize(&path) {
        Ok(p) => Ok(with_host(|h| h.new_str(p.to_string_lossy().into_owned()))),
        // The message names the path as far as it was RESOLVED before the walk
        // failed — on macOS `/var` is a symlink, so node reports
        // `/private/var/…` where the caller wrote `/var/…`. Reporting the
        // argument verbatim names a path the failing `lstat` never saw.
        Err(e) => Err(err_str("realpath", &resolved_prefix(&path), &e)),
    }
}

/// `path` with its deepest EXISTING ancestor canonicalized and the rest of the
/// components appended — what `realpath(3)` has resolved at the point it gives
/// up.
fn resolved_prefix(path: &str) -> String {
    let p = Path::new(path);
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(base) = std::fs::canonicalize(cur) {
            let mut out = base;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out.to_string_lossy().into_owned();
        }
        match (cur.file_name(), cur.parent()) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                rest.push(name);
                cur = parent;
            }
            _ => return path.to_string(),
        }
    }
}

fn rename_impl(args: &[Value]) -> Result<Value, String> {
    let from = arg_str(args, 0);
    let to = arg_str(args, 1);
    match std::fs::rename(&from, &to) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str2("rename", &from, &to, &e)),
    }
}

fn truncate_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let len = arg_num(args, 1);
    let len = if len.is_nan() { 0 } else { len as u64 };
    let r = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|f| f.set_len(len));
    match r {
        Ok(_) => Ok(Value::Undef),
        // The failure is in the `open` this does first, which is the syscall
        // node names — `truncate` only appears once a descriptor exists.
        Err(e) => Err(err_str("open", &path, &e)),
    }
}

fn mkdtemp_impl(args: &[Value]) -> Result<Value, String> {
    let prefix = arg_str(args, 0);
    for _ in 0..64 {
        let candidate = format!("{prefix}{}", random_suffix());
        match std::fs::create_dir(&candidate) {
            Ok(_) => return Ok(with_host(|h| h.new_str(candidate))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(err_str("mkdtemp", &prefix, &e)),
        }
    }
    Err(format!(
        "Error: EEXIST: file already exists, mkdtemp '{prefix}'"
    ))
}

/// Six random `[0-9A-Za-z]` characters — Node's `mkdtemp` suffix alphabet.
fn random_suffix() -> String {
    const CHARS: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut raw = [0u8; 6];
    if getrandom::getrandom(&mut raw).is_err() {
        // Fall back to a clock-derived seed; uniqueness is retried by the caller.
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        raw = nanos
            .to_le_bytes()
            .iter()
            .cycle()
            .take(6)
            .copied()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
    }
    raw.iter()
        .map(|b| CHARS[(*b as usize) % 62] as char)
        .collect()
}

// ── argument validation for the descriptor and mode calls ────────────────────
//
// node checks these in JavaScript (`lib/fs.js`, `internal/fs/utils.js`,
// `internal/validators.js`) and then in the binding (`fd`), always BEFORE any
// I/O, and a callback form throws them synchronously rather than handing them
// to the callback. They used to be read with an infallible `ToNumber`: a
// string mode `'644'` set permissions 0o1204, `writeSync(fd, {})` wrote the 15
// bytes of `[object Object]`, and an unknown flag string opened read-only.

/// The checks one call makes before its I/O, in node's order. `op` is the API
/// name without `Sync`; a FUNCTION in an optional position is the callback of
/// the async form, not the argument.
fn validate_call(op: &str, args: &[Value]) -> Result<(), String> {
    let arg = |i: usize| match args.get(i) {
        Some(v) if !is_fn(v) => v.clone(),
        _ => Value::Undef,
    };
    match op {
        "open" => {
            string_to_flags(&arg(1))?;
            parse_file_mode(&arg(2), "mode", Some(0o666))?;
        }
        "chmod" => {
            parse_file_mode(&arg(1), "mode", None)?;
        }
        "utimes" | "lutimes" => {
            to_unix_timestamp(&arg(1), "atime")?;
            to_unix_timestamp(&arg(2), "mtime")?;
        }
        "close" | "fstat" | "fsync" | "fdatasync" => {
            validate_fd(&arg(0))?;
        }
        "fchmod" => {
            parse_file_mode(&arg(1), "mode", None)?;
            validate_fd(&arg(0))?;
        }
        "fchown" => {
            validate_id(&arg(1), "uid")?;
            validate_id(&arg(2), "gid")?;
            validate_fd(&arg(0))?;
        }
        "futimes" => {
            to_unix_timestamp(&arg(1), "atime")?;
            to_unix_timestamp(&arg(2), "mtime")?;
            validate_fd(&arg(0))?;
        }
        "ftruncate" => {
            let len = arg(1);
            if !matches!(len, Value::Undef) {
                super::validate_integer_in(&len, "len", -MAX_SAFE, MAX_SAFE)?;
            }
            validate_fd(&arg(0))?;
        }
        "readv" | "writev" => {
            validate_buffer_array(&arg(1))?;
            validate_fd(&arg(0))?;
        }
        "write" => {
            validate_write(args)?;
            validate_fd(&arg(0))?;
        }
        _ => {}
    }
    Ok(())
}

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// The path parameters of each call: argument position, the name node's
/// `getValidatedPath(value, name)` reports, and whether a descriptor NUMBER is
/// accepted in its place (`readFileSync(0)`).
fn path_params(op: &str) -> &'static [(usize, &'static str, bool)] {
    match op {
        "readFile" | "writeFile" | "appendFile" | "truncate" => &[(0, "path", true)],
        "access" | "exists" | "chmod" | "chown" | "lchown" | "utimes" | "lutimes" | "mkdir"
        | "open" | "opendir" | "readdir" | "readlink" | "realpath" | "rm" | "rmdir" | "stat"
        | "lstat" | "statfs" | "unlink" | "watchFile" | "unwatchFile" => &[(0, "path", false)],
        "copyFile" | "cp" => &[(0, "src", false), (1, "dest", false)],
        "link" => &[(0, "existingPath", false), (1, "newPath", false)],
        "symlink" => &[(0, "target", false), (1, "path", false)],
        "rename" => &[(0, "oldPath", false), (1, "newPath", false)],
        _ => &[],
    }
}

/// `getValidatedPath` for each path parameter of `op`: a string, a
/// `Uint8Array` (its bytes), or a `file:` URL (its path), with no NUL. The
/// arguments come back with each path replaced by its string, or `None` when
/// `op` takes no path. A URL or Buffer used to be stringified as
/// `[object Object]` and a number or `undefined` opened as the file `5` /
/// `undefined`.
fn validated_paths(op: &str, args: &[Value]) -> Result<Option<Vec<Value>>, String> {
    let params = path_params(op);
    if params.is_empty() {
        return Ok(None);
    }
    let mut out = args.to_vec();
    for &(i, name, fd_ok) in params {
        let v = args.get(i).cloned().unwrap_or(Value::Undef);
        if fd_ok && matches!(v, Value::Float(_) | Value::Int(_)) {
            continue;
        }
        let path = validated_path(&v, name)?;
        if i < out.len() {
            out[i] = with_host(|h| h.new_str(path));
        }
    }
    Ok(Some(out))
}

fn validated_path(v: &Value, name: &str) -> Result<String, String> {
    let tag = native_tag(v);
    let (path, shown) = if tag.as_deref() == Some("URL") {
        let s = super::url::file_url_to_path(std::slice::from_ref(v))?;
        let shown = with_host(|h| h.new_str(s.clone()));
        (s, shown)
    } else if let Some(s) = with_host(|h| h.as_str(v)) {
        (s, v.clone())
    } else if is_uint8_array(v) {
        let bytes = super::buffer::view_bytes(v).unwrap_or_default();
        (String::from_utf8_lossy(&bytes).into_owned(), v.clone())
    } else {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"{name}\" argument must be of type string or an instance of Buffer or URL. Received {}",
                super::received_desc(v)
            ),
        ));
    };
    if path.contains('\0') {
        let kind = if name.contains('.') {
            "property"
        } else {
            "argument"
        };
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            &format!(
                "The {kind} '{name}' must be a string, Uint8Array, or URL without null bytes. Received {}",
                with_host(|h| h.inspect(&shown))
            ),
        ));
    }
    Ok(path)
}

/// `isUint8Array`: a Buffer or a `Uint8Array` (not another element type).
fn is_uint8_array(v: &Value) -> bool {
    match native_tag(v).as_deref() {
        Some("Buffer") => true,
        Some("TypedArray") => super::typedarray::kind_of(v) == "Uint8Array",
        _ => false,
    }
}

/// The binding's descriptor check, `validateInt32(fd, 'fd', 0)`.
fn validate_fd(v: &Value) -> Result<i32, String> {
    validate_int32(v, "fd", 0.0)
}

/// `validateInt32(value, name, min)`: a Number, an integer, in `[min, 2³¹)`.
fn validate_int32(v: &Value, name: &str, min: f64) -> Result<i32, String> {
    if with_host(|h| h.type_of(v)) != "number" {
        return Err(crate::host::invalid_arg_type(name, "argument", "number", v));
    }
    let n = with_host(|h| h.to_number(v));
    if n.fract() != 0.0 || !n.is_finite() {
        return Err(super::out_of_range(name, "an integer", n));
    }
    if n < min || n > 2_147_483_647.0 {
        let must = format!(">= {} && <= 2147483647", crate::host::fmt_number(min));
        return Err(super::out_of_range(name, &must, n));
    }
    Ok(n as i32)
}

/// A `uid`/`gid`: `validateInteger(id, name, -1, kMaxUserId)`.
fn validate_id(v: &Value, name: &str) -> Result<i64, String> {
    super::validate_integer_in(v, name, -1.0, 4_294_967_295.0).map(|n| n as i64)
}

/// `parseFileMode(value, name, def)`: an octal string or a 32-bit unsigned
/// integer, `def` standing in for `undefined`/`null`.
fn parse_file_mode(v: &Value, name: &str, def: Option<u32>) -> Result<u32, String> {
    let nullish = matches!(v, Value::Undef) || with_host(|h| h.is_null(v));
    let v = match (nullish, def) {
        (true, Some(d)) => Value::Float(d as f64),
        (true, None) => Value::Undef,
        (false, _) => v.clone(),
    };
    let mut v = v;
    if let Some(s) = with_host(|h| h.as_str(&v)) {
        if s.is_empty() || !s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                &format!(
                    "The argument '{name}' must be a 32-bit unsigned integer or an octal string. Received {}",
                    with_host(|h| h.inspect(&v))
                ),
            ));
        }
        // `NumberParseInt(value, 8)`, then the range check below.
        let n = s.bytes().fold(0.0, |n, b| n * 8.0 + f64::from(b - b'0'));
        v = Value::Float(n);
    }
    if with_host(|h| h.type_of(&v)) != "number" {
        return Err(crate::host::invalid_arg_type(
            name, "argument", "number", &v,
        ));
    }
    let n = with_host(|h| h.to_number(&v));
    if n.fract() != 0.0 || !n.is_finite() {
        return Err(super::out_of_range(name, "an integer", n));
    }
    if !(0.0..=4_294_967_295.0).contains(&n) {
        return Err(super::out_of_range(name, ">= 0 && <= 4294967295", n));
    }
    Ok(n as u32)
}

/// `stringToFlags(flags)`: a number is the `open(2)` flags themselves, a
/// nullish value is `O_RDONLY`, and the strings are node's fixed table.
fn string_to_flags(v: &Value) -> Result<libc::c_int, String> {
    use libc::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_SYNC, O_TRUNC, O_WRONLY};
    if with_host(|h| h.type_of(v)) == "number" {
        return validate_int32(v, "flags", -2_147_483_648.0);
    }
    if matches!(v, Value::Undef) || with_host(|h| h.is_null(v)) {
        return Ok(O_RDONLY);
    }
    let s = with_host(|h| h.as_str(v));
    Ok(match s.as_deref() {
        Some("r") => O_RDONLY,
        Some("rs" | "sr") => O_RDONLY | O_SYNC,
        Some("r+") => O_RDWR,
        Some("rs+" | "sr+") => O_RDWR | O_SYNC,
        Some("w") => O_TRUNC | O_CREAT | O_WRONLY,
        Some("wx" | "xw") => O_TRUNC | O_CREAT | O_WRONLY | O_EXCL,
        Some("w+") => O_TRUNC | O_CREAT | O_RDWR,
        Some("wx+" | "xw+") => O_TRUNC | O_CREAT | O_RDWR | O_EXCL,
        Some("a") => O_APPEND | O_CREAT | O_WRONLY,
        Some("ax" | "xa") => O_APPEND | O_CREAT | O_WRONLY | O_EXCL,
        Some("as" | "sa") => O_APPEND | O_CREAT | O_WRONLY | O_SYNC,
        Some("a+") => O_APPEND | O_CREAT | O_RDWR,
        Some("ax+" | "xa+") => O_APPEND | O_CREAT | O_RDWR | O_EXCL,
        Some("as+" | "sa+") => O_APPEND | O_CREAT | O_RDWR | O_SYNC,
        _ => {
            return Err(crate::host::coded_error(
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                &format!(
                    "The argument 'flags' is invalid. Received {}",
                    with_host(|h| h.inspect(v))
                ),
            ))
        }
    })
}

/// `toUnixTimestamp(time, name)`: seconds from a numeric string, a finite
/// number (a negative one meaning NOW), or a `Date`.
fn to_unix_timestamp(v: &Value, name: &str) -> Result<f64, String> {
    // `+time == time` holds for exactly the strings that convert to a number.
    if with_host(|h| h.as_str(v).is_some()) {
        let n = with_host(|h| h.to_number(v));
        if !n.is_nan() {
            return Ok(n);
        }
    }
    if matches!(v, Value::Float(_) | Value::Int(_)) {
        let n = with_host(|h| h.to_number(v));
        if n.is_finite() {
            if n < 0.0 {
                let now = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);
                return Ok(now / 1000.0);
            }
            return Ok(n);
        }
    }
    if native_tag(v).as_deref() == Some("Date") {
        return Ok(super::date::ms_of(v) / 1000.0);
    }
    Err(crate::host::coded_error(
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"{name}\" argument must be an instance of Date or an Time in seconds. Received {}",
            super::received_desc(v)
        ),
    ))
}

/// `validateBufferArray(buffers)`: an Array whose every element is a byte view.
fn validate_buffer_array(v: &Value) -> Result<(), String> {
    let items = with_host(|h| match h.get(v) {
        Some(JsObj::Array(items)) => Some(items.clone()),
        _ => None,
    });
    let all_views = items.as_ref().is_some_and(|items| {
        items.iter().all(|b| {
            matches!(
                native_tag(b).as_deref(),
                Some("Buffer" | "TypedArray" | "DataView")
            )
        })
    });
    if all_views {
        return Ok(());
    }
    Err(crate::host::coded_error(
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"buffers\" argument must be an ArrayBufferView[]. Received {}",
            super::received_desc(v)
        ),
    ))
}

/// `writeSync`'s own checks (`lib/fs.js`): for a byte view, `offset` an
/// integer `>= 0` and `validateOffsetLengthWrite`; otherwise the data must be
/// a string (`validateStringAfterArrayBufferView`).
fn validate_write(args: &[Value]) -> Result<(), String> {
    let src = args.get(1).cloned().unwrap_or(Value::Undef);
    let Some(len) = super::buffer::view_bytes(&src).map(|b| b.len() as f64) else {
        if with_host(|h| h.as_str(&src).is_some()) {
            return Ok(());
        }
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"buffer\" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received {}",
                super::received_desc(&src)
            ),
        ));
    };
    let third = args.get(2).cloned().unwrap_or(Value::Undef);
    let (offset, length) = if with_host(|h| h.type_of(&third)) == "object" {
        let opts = third;
        let nullish = with_host(|h| h.is_null(&opts));
        let get = |k: &str| match nullish {
            true => Ok(Value::Undef),
            false => crate::builtins::get_property(&opts, k),
        };
        (get("offset")?, get("length")?)
    } else {
        (third, args.get(3).cloned().unwrap_or(Value::Undef))
    };
    let offset = if matches!(offset, Value::Undef) || with_host(|h| h.is_null(&offset)) {
        0.0
    } else {
        super::validate_integer(&offset, "offset", 0.0)?
    };
    let length = match length {
        Value::Float(n) => n,
        Value::Int(n) => n as f64,
        _ => len - offset,
    };
    // `validateOffsetLengthWrite`.
    if offset > len {
        return Err(super::out_of_range(
            "offset",
            &format!("<= {}", crate::host::fmt_number(len)),
            offset,
        ));
    }
    if length > len - offset {
        let must = format!("<= {}", crate::host::fmt_number(len - offset));
        return Err(super::out_of_range("length", &must, length));
    }
    if length < 0.0 {
        return Err(super::out_of_range("length", ">= 0", length));
    }
    Ok(())
}

// ── file descriptors ─────────────────────────────────────────────────────────

/// `fs.openSync(path, flags, mode)`: one `open(2)` with node's flag table and
/// `mode` (so `wx` really is exclusive, `as` really is synchronous, and a
/// numeric flag set or a created file's mode is honoured).
fn open_impl(args: &[Value]) -> Result<Value, String> {
    let arg = |i: usize| match args.get(i) {
        Some(v) if !is_fn(v) => v.clone(),
        _ => Value::Undef,
    };
    let flags = string_to_flags(&arg(1))?;
    let mode = parse_file_mode(&arg(2), "mode", Some(0o666))?;
    let path = arg_str(args, 0);
    let file = open_file(&path, flags, mode)?;
    Ok(Value::Float(register_fd(file) as f64))
}

/// One `open(2)` of `path`, its failure worded as node words an `open`.
fn open_file(path: &str, flags: libc::c_int, mode: u32) -> Result<File, String> {
    use std::os::unix::io::FromRawFd;
    let c = cpath(path, "open")?;
    let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC, mode as libc::c_uint) };
    if fd < 0 {
        return Err(err_str("open", path, &std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn close_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    if close_fd(fd) {
        Ok(Value::Undef)
    } else {
        Err("Error: EBADF: bad file descriptor, close".to_string())
    }
}

/// `fs.readSync(fd, buffer[, offset[, length[, position]]])` and its options
/// form, a port of `lib/fs.js`. Validation errors are the OUTER error (they
/// throw, even from the callback form); the read's own failure is the inner.
fn read_impl(args: &[Value]) -> Result<usize, String> {
    let buffer = args.get(1).cloned().unwrap_or(Value::Undef);
    let third = args.get(2).cloned().unwrap_or(Value::Undef);
    let req = if args.len() <= 3 || with_host(|h| h.type_of(&third)) == "object" {
        read_options(&buffer, &third)?
    } else {
        ReadArgs {
            offset: third,
            length: args.get(3).cloned().unwrap_or(Value::Undef),
            position: args.get(4).cloned().unwrap_or(Value::Undef),
        }
    };
    read_validated(args.first().unwrap_or(&Value::Undef), &buffer, &req)?
}

/// The three operands of a read, before validation.
struct ReadArgs {
    offset: Value,
    length: Value,
    position: Value,
}

/// The `{offset = 0, length = buffer.byteLength - offset, position = null}`
/// destructuring of a read's options object (`null`/`undefined` mean `{}`).
fn read_options(buffer: &Value, opts: &Value) -> Result<ReadArgs, String> {
    let nullish = matches!(opts, Value::Undef) || with_host(|h| h.is_null(opts));
    if !nullish && with_host(|h| h.type_of(opts)) != "object" {
        return Err(crate::host::invalid_arg_type(
            "options", "argument", "object", opts,
        ));
    }
    let field = |k: &str| -> Result<Value, String> {
        if nullish {
            Ok(Value::Undef)
        } else {
            crate::builtins::get_property(opts, k)
        }
    };
    let offset = match field("offset")? {
        Value::Undef => Value::Float(0.0),
        v => v,
    };
    let length = match field("length")? {
        Value::Undef => {
            let len = super::buffer::view_bytes(buffer).map_or(0, |b| b.len()) as f64;
            Value::Float(len - with_host(|h| h.to_number(&offset)))
        }
        v => v,
    };
    let position = field("position")?;
    Ok(ReadArgs {
        offset,
        length,
        position,
    })
}

/// Validate a read's operands as `readSync` does, then read. The outer error
/// is a validation failure; the inner one the read itself. The bytes land at
/// `offset` BYTES into whatever view `buffer` is — a `Uint16Array` or a
/// `DataView` receives them exactly as a Buffer does — and a `position` reads
/// with `pread`, leaving the descriptor's own offset where it was.
fn read_validated(
    fd: &Value,
    buffer: &Value,
    req: &ReadArgs,
) -> Result<Result<usize, String>, String> {
    validate_read_buffer(buffer)?;
    let offset = match req.offset {
        Value::Undef => 0.0,
        ref v => super::validate_integer(v, "offset", 0.0)?,
    };
    let length = crate::host::to_number_value(&req.length)?;
    let length = crate::host::to_int32(length) as f64;
    if length == 0.0 {
        return Ok(Ok(0));
    }
    let mut bytes = super::buffer::view_bytes(buffer).unwrap_or_default();
    if bytes.is_empty() {
        return Err(crate::host::coded_error(
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            &format!(
                "The argument 'buffer' is empty and cannot be written. Received {}",
                with_host(|h| h.inspect(buffer))
            ),
        ));
    }
    if length < 0.0 {
        return Err(super::out_of_range("length", ">= 0", length));
    }
    let cap = bytes.len() as f64;
    if offset + length > cap {
        return Err(super::out_of_range(
            "length",
            &format!("<= {}", cap - offset),
            length,
        ));
    }
    let position = read_position(&req.position, length)?;
    // The binding checks the descriptor last, once the JavaScript-side checks
    // above have passed.
    let fd = validate_fd(fd)?;
    let (offset, length) = (offset as usize, length as usize);
    let dest = &mut bytes[offset..offset + length];
    let got = if fd == 0 && position.is_none() {
        std::io::stdin().lock().read(dest)
    } else {
        use std::os::unix::fs::FileExt;
        match with_file(fd, |file| match position {
            Some(pos) => file.read_at(dest, pos),
            None => {
                let mut fr: &File = file;
                fr.read(dest)
            }
        }) {
            Some(r) => r,
            None => return Ok(Err("Error: EBADF: bad file descriptor, read".to_string())),
        }
    };
    Ok(match got {
        Ok(n) => {
            super::buffer::write_view(buffer, &bytes);
            Ok(n)
        }
        Err(e) => Err(err_str("read", "", &e)),
    })
}

/// node's `validatePosition`: `null`/`undefined`/`-1` read from the current
/// offset; otherwise an integer or a bigint at least `-1`.
fn read_position(v: &Value, length: f64) -> Result<Option<u64>, String> {
    if matches!(v, Value::Undef) || with_host(|h| h.is_null(v)) {
        return Ok(None);
    }
    let pos = match with_host(|h| h.type_of(v)) {
        "number" => super::validate_integer(v, "position", -1.0)?,
        "bigint" => {
            let p: i128 = with_host(|h| h.str_of(v)).parse().unwrap_or(-2);
            let max = i64::MAX as i128 - length as i128;
            if !(-1..=max).contains(&p) {
                return Err(crate::host::coded_error(
                    "RangeError",
                    "ERR_OUT_OF_RANGE",
                    &format!(
                        "The value of \"position\" is out of range. It must be >= -1n && <= {max}n. Received {p}n"
                    ),
                ));
            }
            p as f64
        }
        _ => {
            return Err(crate::host::invalid_arg_type(
                "position",
                "argument",
                "bigint or integer",
                v,
            ))
        }
    };
    Ok((pos >= 0.0).then_some(pos as u64))
}

/// `validateBuffer`: a read's destination must be a Buffer, typed array or
/// `DataView` — an `ArrayBuffer` is not a view.
fn validate_read_buffer(buffer: &Value) -> Result<(), String> {
    if matches!(
        native_tag(buffer).as_deref(),
        Some("Buffer" | "TypedArray" | "DataView")
    ) {
        return Ok(());
    }
    Err(crate::host::coded_error(
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"buffer\" argument must be an instance of Buffer, TypedArray, or DataView. Received {}",
            super::received_desc(buffer)
        ),
    ))
}

/// `fs.read`, whose callback is `(err, bytesRead, buffer)`. The argument forms
/// are `lib/fs.js`'s, counted with the descriptor: `(fd, cb)` reads into a fresh
/// 16 KiB Buffer, `(fd, bufferOrOptions, cb)`, `(fd, buffer, options, cb)`, and
/// the positional `(fd, buffer, offset, length, position, cb)`. A bad argument
/// throws here, synchronously; only the read's own failure reaches `cb`.
fn read_async(args: &[Value]) -> Result<Value, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Undef);
    let fresh = || super::buffer::from_bytes(&[0u8; 16384]);
    let (buffer, req, cb) = match args.len() {
        0..=2 => {
            let buffer = fresh();
            let req = read_options(&buffer, &Value::Undef)?;
            (buffer, req, arg(1))
        }
        3 if !matches!(
            native_tag(&arg(1)).as_deref(),
            Some("Buffer" | "TypedArray" | "DataView")
        ) =>
        {
            let params = arg(1);
            let nullish = matches!(params, Value::Undef) || with_host(|h| h.is_null(&params));
            let buffer = match nullish {
                true => Value::Undef,
                false => crate::builtins::get_property(&params, "buffer")?,
            };
            let buffer = if matches!(buffer, Value::Undef) {
                fresh()
            } else {
                buffer
            };
            let req = read_options(&buffer, &params)?;
            (buffer, req, arg(2))
        }
        3 => (arg(1), read_options(&arg(1), &Value::Undef)?, arg(2)),
        4 => (arg(1), read_options(&arg(1), &arg(2))?, arg(3)),
        _ => (
            arg(1),
            ReadArgs {
                offset: arg(2),
                length: arg(3),
                position: arg(4),
            },
            arg(5),
        ),
    };
    validate_read_buffer(&buffer)?;
    if !is_fn(&cb) {
        return Err(crate::host::invalid_arg_type(
            "cb", "argument", "function", &cb,
        ));
    }
    let result = read_validated(args.first().unwrap_or(&Value::Undef), &buffer, &req)?;
    with_host(|h| match result {
        Ok(n) => {
            let nul = h.null();
            h.queue_io_callback(cb, vec![nul, Value::Float(n as f64), buffer]);
        }
        Err(e) => {
            let ev = crate::builtins::synth_error(h, &e);
            h.queue_io_callback(cb, vec![ev]);
        }
    });
    Ok(Value::Undef)
}

fn write_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let src = args.get(1).cloned().unwrap_or(Value::Undef);
    // The positional form differs for a byte VIEW and a string, so the two are
    // distinguished by what the argument IS, not by whether it is a Buffer.
    let view = super::buffer::view_bytes(&src);
    // Buffer form: write(fd, buffer, offset, length, position)
    // String form: write(fd, string, position, encoding)
    let (data, position) = if let Some(all) = view {
        let offset = num_or(args, 2, 0.0) as usize;
        let length = num_or(args, 3, (all.len().saturating_sub(offset)) as f64) as usize;
        let end = (offset + length).min(all.len());
        (
            all[offset.min(all.len())..end].to_vec(),
            position_arg(args, 4),
        )
    } else {
        (
            with_host(|h| h.str_of(&src)).into_bytes(),
            position_arg(args, 2),
        )
    };
    // A numeric `position` is `pwrite`: the descriptor's offset stays where it
    // was, so a later position-less write continues from there.
    let r = with_file(fd, |file| write_maybe_at(file, &data, position));
    match r {
        Some(Ok(n)) => Ok(n),
        Some(Err(e)) => Err(err_str("write", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, write".to_string()),
    }
}

/// `fs.readvSync(fd, buffers[, position])`: fill each view in turn, stopping at
/// end of file. Any byte view takes part (a `Uint8Array` used to be skipped as
/// zero-capacity), and a numeric `position` reads with `pread`, leaving the
/// descriptor's offset where it was.
fn readv_impl(args: &[Value]) -> Result<usize, String> {
    use std::os::unix::fs::FileExt;
    let fd = arg_num(args, 0) as i32;
    let buffers = array_items(args.get(1));
    let mut position = position_arg(args, 2);
    let mut total = 0;
    for b in &buffers {
        let mut bytes = super::buffer::view_bytes(b).unwrap_or_default();
        let got = with_file(fd, |file| match position {
            Some(pos) => file.read_at(&mut bytes, pos),
            None => {
                let mut fr: &File = file;
                fr.read(&mut bytes)
            }
        });
        let n = match got {
            Some(Ok(n)) => n,
            // libuv reports a failed vectored read as the `read` it issues.
            Some(Err(e)) => return Err(err_str("read", "", &e)),
            None => return Err("Error: EBADF: bad file descriptor, read".to_string()),
        };
        if n == 0 {
            break;
        }
        super::buffer::write_view(b, &bytes);
        total += n;
        position = position.map(|p| p + n as u64);
        if n < bytes.len() {
            break;
        }
    }
    Ok(total)
}

fn writev_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let buffers = array_items(args.get(1));
    let position = position_arg(args, 2);
    let mut data = Vec::new();
    for b in &buffers {
        data.extend(buf_bytes(b));
    }
    // `pwritev` for a numeric position, as in `writeSync`. libuv reports a
    // failed vectored write as the `write` it issues.
    let r = with_file(fd, |file| write_maybe_at(file, &data, position));
    match r {
        Some(Ok(n)) => Ok(n),
        Some(Err(e)) => Err(err_str("write", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, write".to_string()),
    }
}

/// One `write(2)` at the descriptor's offset, or one `pwrite(2)` at `position`.
fn write_maybe_at(file: &File, data: &[u8], position: Option<u64>) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    match position {
        Some(pos) => file.write_at(data, pos),
        None => {
            let mut fr: &File = file;
            fr.write(data)
        }
    }
}

fn fstat_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let md = with_file(fd, |file| file.metadata());
    match md {
        Some(Ok(md)) => Ok(with_host(|h| build_stats(h, &md))),
        Some(Err(e)) => Err(err_str("fstat", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, fstat".to_string()),
    }
}

fn fchmod_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let mode = parse_file_mode(args.get(1).unwrap_or(&Value::Undef), "mode", None)? as libc::mode_t;
    let rc = with_file(fd, |file| unsafe { libc::fchmod(file.as_raw_fd(), mode) });
    fd_result(rc, "fchmod")
}

fn fchown_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let uid = arg_num(args, 1) as libc::uid_t;
    let gid = arg_num(args, 2) as libc::gid_t;
    let rc = with_file(fd, |file| unsafe {
        libc::fchown(file.as_raw_fd(), uid, gid)
    });
    fd_result(rc, "fchown")
}

fn futimes_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let times = [
        to_timeval(time_secs(args, 1)),
        to_timeval(time_secs(args, 2)),
    ];
    let rc = with_file(fd, |file| unsafe {
        libc::futimes(file.as_raw_fd(), times.as_ptr())
    });
    fd_result(rc, "futimes")
}

fn ftruncate_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let len = arg_num(args, 1);
    let len = if len.is_nan() { 0 } else { len as u64 };
    match with_file(fd, |file| file.set_len(len)) {
        Some(Ok(_)) => Ok(Value::Undef),
        Some(Err(e)) => Err(err_str("ftruncate", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, ftruncate".to_string()),
    }
}

fn fsync_impl(args: &[Value], data_only: bool) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    let op = if data_only { "fdatasync" } else { "fsync" };
    let r = with_file(fd, |file| {
        if data_only {
            file.sync_data()
        } else {
            file.sync_all()
        }
    });
    match r {
        Some(Ok(_)) => Ok(Value::Undef),
        Some(Err(e)) => Err(err_str(op, "", &e)),
        None => Err(format!("Error: EBADF: bad file descriptor, {op}")),
    }
}

/// Translate the return of a `with_file` libc call: `None` = bad fd, `Some(0)` =
/// success, `Some(_)` = the C failure whose reason is in `errno`.
fn fd_result(rc: Option<libc::c_int>, op: &str) -> Result<Value, String> {
    match rc {
        Some(0) => Ok(Value::Undef),
        Some(_) => Err(err_str(op, "", &std::io::Error::last_os_error())),
        None => Err(format!("Error: EBADF: bad file descriptor, {op}")),
    }
}

// ── stat / statfs ────────────────────────────────────────────────────────────

fn stat_impl(op: &str, args: &[Value], follow: bool) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let md = if follow {
        std::fs::metadata(&path)
    } else {
        std::fs::symlink_metadata(&path)
    };
    match md {
        Ok(md) => Ok(with_host(|h| build_stats(h, &md))),
        Err(e) => Err(err_str(op, &path, &e)),
    }
}

fn statfs_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let c = cpath(&path, "statfs")?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(err_str("statfs", &path, &std::io::Error::last_os_error()));
    }
    Ok(with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("type".into(), Value::Float(st.f_fsid as f64));
        m.insert("bsize".into(), Value::Float(st.f_bsize as f64));
        m.insert("blocks".into(), Value::Float(st.f_blocks as f64));
        m.insert("bfree".into(), Value::Float(st.f_bfree as f64));
        m.insert("bavail".into(), Value::Float(st.f_bavail as f64));
        m.insert("files".into(), Value::Float(st.f_files as f64));
        m.insert("ffree".into(), Value::Float(st.f_ffree as f64));
        h.new_object(m)
    }))
}

/// Build a full `fs.Stats` object from `Metadata` (unix fields via `MetadataExt`).
fn build_stats(h: &mut crate::host::JsHost, md: &std::fs::Metadata) -> Value {
    let ns = |s: i64, n: i64| s as f64 * 1000.0 + n as f64 / 1_000_000.0;
    let atime_ms = ns(md.atime(), md.atime_nsec());
    let mtime_ms = ns(md.mtime(), md.mtime_nsec());
    let ctime_ms = ns(md.ctime(), md.ctime_nsec());
    let birth_ms = md
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(mtime_ms);
    let ft = md.file_type();
    let date = |h: &mut crate::host::JsHost, ms: f64| {
        let mut d = IndexMap::new();
        d.insert("@@native".into(), h.new_str("Date"));
        d.insert("@@ms".into(), Value::Float(ms));
        h.new_object(d)
    };
    let mut m = IndexMap::new();
    m.insert("@@native".into(), h.new_str("Stats"));
    m.insert("@@isFile".into(), Value::Bool(md.is_file()));
    m.insert("@@isDir".into(), Value::Bool(md.is_dir()));
    m.insert("@@isSymlink".into(), Value::Bool(ft.is_symlink()));
    m.insert("dev".into(), Value::Float(md.dev() as f64));
    m.insert("mode".into(), Value::Float(md.mode() as f64));
    m.insert("nlink".into(), Value::Float(md.nlink() as f64));
    m.insert("uid".into(), Value::Float(md.uid() as f64));
    m.insert("gid".into(), Value::Float(md.gid() as f64));
    m.insert("rdev".into(), Value::Float(md.rdev() as f64));
    m.insert("blksize".into(), Value::Float(md.blksize() as f64));
    m.insert("ino".into(), Value::Float(md.ino() as f64));
    m.insert("size".into(), Value::Float(md.len() as f64));
    m.insert("blocks".into(), Value::Float(md.blocks() as f64));
    m.insert("atimeMs".into(), Value::Float(atime_ms));
    m.insert("mtimeMs".into(), Value::Float(mtime_ms));
    m.insert("ctimeMs".into(), Value::Float(ctime_ms));
    m.insert("birthtimeMs".into(), Value::Float(birth_ms));
    let atime = date(h, atime_ms);
    m.insert("atime".into(), atime);
    let mtime = date(h, mtime_ms);
    m.insert("mtime".into(), mtime);
    let ctime = date(h, ctime_ms);
    m.insert("ctime".into(), ctime);
    let birthtime = date(h, birth_ms);
    m.insert("birthtime".into(), birthtime);
    let obj = h.new_object(m);
    // The four Date fields are ACCESSORS on `Stats.prototype` in node, so they
    // are not own enumerable keys: `Object.keys(statSync(f))` is the fourteen
    // numeric fields and nothing else. They are stored here rather than
    // computed, so the equivalent is to hide them — a `JSON.stringify(stat)`
    // or an `{...stat}` carried four extra entries node does not.
    for k in ["atime", "mtime", "ctime", "birthtime"] {
        h.hide_prop(&obj, k);
    }
    obj
}

// ── watchFile / unwatchFile ──────────────────────────────────────────────────

fn watch_file(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let Some(listener) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    let interval = interval_opt(args).unwrap_or(5007.0).max(1.0) as u64;
    let abs = std::fs::canonicalize(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.clone());

    let stop = Arc::new(AtomicBool::new(false));
    let id = NEXT_WATCH_ID.with(|n| {
        let v = *n.borrow();
        *n.borrow_mut() = v + 1;
        v
    });
    WATCHERS.with(|w| {
        w.borrow_mut().push(WatchEntry {
            id,
            path: abs.clone(),
            listener: listener.clone(),
            stop: stop.clone(),
        });
    });
    with_host(|h| h.incr_handle());

    let tx = with_host(|h| h.io_sender());
    let poll_stop = stop.clone();
    let poll_listener = listener;
    std::thread::spawn(move || {
        let mut prev = stat_parts(&abs);
        loop {
            if poll_stop.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(interval));
            if poll_stop.load(Ordering::Acquire) {
                break;
            }
            let curr = stat_parts(&abs);
            if curr != prev {
                let l = poll_listener.clone();
                let (p0, p1, p2) = prev;
                let (c0, c1, c2) = curr;
                let _ = tx.send(Box::new(move || {
                    let cur = with_host(|h| stats_from_parts(h, c0, c1, c2));
                    let old = with_host(|h| stats_from_parts(h, p0, p1, p2));
                    if let Err(e) = invoke(&l, vec![cur, old], None) {
                        eprintln!("{e}");
                    }
                    Ok(())
                }));
                prev = curr;
            }
        }
    });
    Ok(Value::Undef)
}

fn unwatch_file(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let abs = std::fs::canonicalize(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.clone());
    let listener = args.get(1).cloned().filter(is_fn);
    let removed = WATCHERS.with(|w| {
        let mut w = w.borrow_mut();
        let mut count = 0;
        w.retain(|e| {
            let matches =
                e.path == abs && listener.as_ref().map(|l| *l == e.listener).unwrap_or(true);
            if matches {
                e.stop.store(true, Ordering::Release);
                count += 1;
            }
            !matches
        });
        count
    });
    for _ in 0..removed {
        with_host(|h| h.decr_handle());
    }
    // Wake the event loop so it can re-evaluate the handle count.
    if removed > 0 {
        let _ = with_host(|h| h.io_sender()).send(Box::new(|| Ok(())));
    }
    Ok(Value::Undef)
}

/// A file's watch-relevant state: `(exists, mtime_ms, size)`.
fn stat_parts(path: &str) -> (bool, i64, u64) {
    match std::fs::metadata(path) {
        Ok(md) => (
            true,
            md.mtime() * 1000 + md.mtime_nsec() / 1_000_000,
            md.len(),
        ),
        Err(_) => (false, 0, 0),
    }
}

fn stats_from_parts(h: &mut crate::host::JsHost, exists: bool, mtime_ms: i64, size: u64) -> Value {
    let ms = mtime_ms as f64;
    let date = |h: &mut crate::host::JsHost, ms: f64| {
        let mut d = IndexMap::new();
        d.insert("@@native".into(), h.new_str("Date"));
        d.insert("@@ms".into(), Value::Float(ms));
        h.new_object(d)
    };
    let mut m = IndexMap::new();
    m.insert("@@native".into(), h.new_str("Stats"));
    m.insert("@@isFile".into(), Value::Bool(exists));
    m.insert("@@isDir".into(), Value::Bool(false));
    m.insert("@@isSymlink".into(), Value::Bool(false));
    m.insert(
        "size".into(),
        Value::Float(if exists { size as f64 } else { 0.0 }),
    );
    m.insert("atimeMs".into(), Value::Float(ms));
    m.insert("mtimeMs".into(), Value::Float(ms));
    m.insert("ctimeMs".into(), Value::Float(ms));
    m.insert("birthtimeMs".into(), Value::Float(ms));
    let mt = date(h, ms);
    m.insert("mtime".into(), mt);
    let at = date(h, ms);
    m.insert("atime".into(), at);
    h.new_object(m)
}

// ── read / write streams ─────────────────────────────────────────────────────

fn create_read_stream(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let enc = encoding_arg(args, 1);
    let stream = with_host(|h| {
        let mut extra = IndexMap::new();
        extra.insert("path".into(), h.new_str(path.clone()));
        if let Some(e) = &enc {
            extra.insert("@@encoding".into(), h.new_str(e.clone()));
        }
        extra
    });
    let stream = super::net::new_emitter_object("FSReadStream", stream);
    with_host(|h| h.incr_handle());
    // `{ fd }`: the stream reads an already-open descriptor from its current
    // offset and opens nothing (so no `open`/`ready`). `autoClose` (default
    // true) closes it after `end`; with it off the stream never emits `close`.
    let fd = option_value(args, "fd").filter(|v| matches!(v, Value::Float(_) | Value::Int(_)));
    let auto_close = option_value(args, "autoClose").map_or(true, |v| with_host(|h| h.truthy(&v)));
    let recv = stream.clone();
    let p = path;
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            match fd {
                Some(fd) => read_fd_stream_pump(&recv, arg_num(&[fd], 0) as i32, auto_close),
                None => read_stream_pump(&recv, &p),
            }
            Ok(())
        }))
    });
    Ok(stream)
}

/// The value of `key` in the options object among `args`, if present.
fn option_value(args: &[Value], key: &str) -> Option<Value> {
    with_host(|h| {
        args.iter().find_map(|v| match h.get(v) {
            Some(JsObj::Object(p)) => p.get(key).cloned(),
            _ => None,
        })
    })
}

/// `read_stream_pump` for a stream over a caller's descriptor.
fn read_fd_stream_pump(recv: &Value, fd: i32, auto_close: bool) {
    with_host(|h| h.decr_handle());
    let read = with_file(fd, |file| {
        let mut bytes = Vec::new();
        let mut fr: &File = file;
        fr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let bytes = match read {
        Some(Ok(b)) => b,
        failed => {
            let msg = match failed {
                Some(Err(e)) => err_str("read", "", &e),
                _ => "Error: EBADF: bad file descriptor, read".to_string(),
            };
            let ev = with_host(|h| crate::builtins::synth_error(h, &msg));
            let _ = super::events::instance_call(
                recv,
                "emit",
                vec![with_host(|h| h.new_str("error")), ev],
            );
            return;
        }
    };
    emit_stream_bytes(recv, &bytes);
    let target = recv.clone();
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            let end = with_host(|h| h.new_str("end"));
            super::events::instance_call(&target, "emit", vec![end])?;
            if !auto_close {
                return Ok(());
            }
            close_fd(fd);
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    let close = with_host(|h| h.new_str("close"));
                    super::events::instance_call(&target, "emit", vec![close]).map(|_| ())
                }))
            });
            Ok(())
        }))
    });
}

/// Hand a read stream's bytes to its pipe destination, or emit them as one
/// `data` chunk in the stream's encoding.
fn emit_stream_bytes(recv: &Value, bytes: &[u8]) {
    let enc = get_prop(recv, "@@encoding").map(|v| with_host(|h| h.str_of(&v)));
    let chunk = match enc.as_deref() {
        Some(e) if e != "buffer" => {
            with_host(|h| h.new_str(String::from_utf8_lossy(bytes).into_owned()))
        }
        _ => super::buffer::from_bytes(bytes),
    };
    if let Some(dest) = get_prop(recv, "@@pipeDest") {
        let _ = crate::host::call_method(&dest, "write", vec![chunk]);
        let _ = crate::host::call_method(&dest, "end", vec![]);
    } else {
        let name = with_host(|h| h.new_str("data"));
        let _ = super::events::instance_call(recv, "emit", vec![name, chunk]);
    }
}

/// Read the whole file and drive the stream's events on the microtask tick after
/// creation (so synchronously-attached `on('data')`/`on('end')` listeners fire).
fn read_stream_pump(recv: &Value, path: &str) {
    with_host(|h| h.decr_handle());
    let opened = File::open(path).and_then(|f| {
        let mut bytes = Vec::new();
        (&f).read_to_end(&mut bytes).map(|_| (f, bytes))
    });
    let bytes = match opened {
        Ok((file, b)) => {
            // `open` hands listeners the descriptor, then `ready`; the
            // descriptor is real and closed again once the data is out.
            let fd = register_fd(file);
            let open = with_host(|h| h.new_str("open"));
            let _ = super::events::instance_call(recv, "emit", vec![open, Value::Float(fd as f64)]);
            let _ =
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("ready"))]);
            close_fd(fd);
            b
        }

        Err(e) => {
            let ev = with_host(|h| crate::builtins::synth_error(h, &err_str("open", path, &e)));
            let _ = super::events::instance_call(
                recv,
                "emit",
                vec![with_host(|h| h.new_str("error")), ev],
            );
            return;
        }
    };
    emit_stream_bytes(recv, &bytes);
    // `end` and then `close` each follow on a later tick, as node schedules
    // them, so what a `data` listener queued runs before the stream ends.
    let target = recv.clone();
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            let end = with_host(|h| h.new_str("end"));
            super::events::instance_call(&target, "emit", vec![end])?;
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    let close = with_host(|h| h.new_str("close"));
                    super::events::instance_call(&target, "emit", vec![close]).map(|_| ())
                }))
            });
            Ok(())
        }))
    });
}

pub const READ_STREAM_METHODS: &[&str] = &[
    "pipe",
    "pause",
    "resume",
    "setEncoding",
    "destroy",
    "close",
    "read",
];

/// `fs.ReadStream` instance dispatch (tag `FSReadStream`). The EventEmitter methods (`on`, `once`, `emit`, …) delegate to `events`, as every
/// other emitter-backed instance does; this covers the stream-only surface.
/// Nothing routed them here, so `fs.createReadStream(p).on('data', …)` threw
/// `on is not a function`.
pub fn read_stream_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if !READ_STREAM_METHODS.contains(&method) && super::events::METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        "pipe" => {
            if let Some(dest) = args.first().cloned() {
                set_prop(recv, "@@pipeDest", dest.clone());
                Ok(dest)
            } else {
                Ok(recv.clone())
            }
        }
        "setEncoding" => {
            set_prop(
                recv,
                "@@encoding",
                with_host(|h| h.new_str(super::arg_str(&args, 0))),
            );
            Ok(recv.clone())
        }
        "pause" | "resume" | "read" => Ok(recv.clone()),
        "destroy" | "close" => {
            let _ =
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))]);
            Ok(recv.clone())
        }
        _ => Err(crate::host::type_error(&format!(
            "stream.{method} is not a function"
        ))),
    }
}

fn create_write_stream(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let flags = match encoding_flag(args, "flags") {
        Some(f) => f,
        None => "w".to_string(),
    };
    // `{ fd }` writes to an already-open descriptor and opens nothing (no
    // `open`/`ready`); `autoClose: false` leaves it open after `finish`, and
    // the stream then never emits `close`.
    let given = option_value(args, "fd").filter(|v| matches!(v, Value::Float(_) | Value::Int(_)));
    let auto_close = option_value(args, "autoClose").map_or(true, |v| with_host(|h| h.truthy(&v)));
    let fd = match &given {
        Some(v) => arg_num(std::slice::from_ref(v), 0) as i32,
        None => {
            let flags = string_to_flags(&with_host(|h| h.new_str(flags)))?;
            register_fd(open_file(&path, flags, 0o666)?)
        }
    };
    let stream = with_host(|h| {
        let mut extra = IndexMap::new();
        extra.insert("path".into(), h.new_str(path));
        extra.insert("@@wfd".into(), Value::Float(fd as f64));
        extra.insert("@@autoClose".into(), Value::Bool(auto_close));
        extra.insert("bytesWritten".into(), Value::Float(0.0));
        extra
    });
    let stream = super::net::new_emitter_object("FSWriteStream", stream);
    with_host(|h| h.incr_handle());
    if given.is_some() {
        return Ok(stream);
    }
    let recv = stream.clone();
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            let open = with_host(|h| h.new_str("open"));
            let _ =
                super::events::instance_call(&recv, "emit", vec![open, Value::Float(fd as f64)]);
            let _ = super::events::instance_call(
                &recv,
                "emit",
                vec![with_host(|h| h.new_str("ready"))],
            );
            Ok(())
        }))
    });
    Ok(stream)
}

pub const WRITE_STREAM_METHODS: &[&str] = &[
    "write",
    "end",
    "destroy",
    "close",
    "cork",
    "uncork",
    "setDefaultEncoding",
];

/// `fs.WriteStream` instance dispatch (tag `FSWriteStream`), with the
/// EventEmitter methods delegated to `events` as for a read stream.
pub fn write_stream_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    if !WRITE_STREAM_METHODS.contains(&method) && super::events::METHODS.contains(&method) {
        return super::events::instance_call(recv, method, args);
    }
    match method {
        "write" => {
            write_stream_bytes(recv, args.first());
            // The write callback follows the call, with `null` for success.
            if let Some(cb) = args.iter().find(|v| is_fn(v)).cloned() {
                with_host(|h| {
                    h.queue_micro_native(Box::new(move || {
                        let null = with_host(|h| h.null());
                        invoke(&cb, vec![null], None).map(|_| ())
                    }))
                });
            }
            Ok(Value::Bool(true))
        }
        "end" => {
            if let Some(chunk) = args
                .first()
                .filter(|v| !matches!(v, Value::Undef) && !is_fn(v))
            {
                write_stream_bytes(recv, Some(chunk));
            }
            let auto_close =
                get_prop(recv, "@@autoClose").map_or(true, |v| with_host(|h| h.truthy(&v)));
            if let Some(fd) = get_prop(recv, "@@wfd")
                .filter(|_| auto_close)
                .map(|v| with_host(|h| h.to_number(&v)) as i32)
            {
                close_fd(fd);
            }
            with_host(|h| h.decr_handle());
            // `finish`, the `end` callback and `close` follow the call rather
            // than happening inside it: node finishes a write stream once its
            // pending writes have drained, so code after `end()` runs first.
            let target = recv.clone();
            let cb = args.iter().find(|v| is_fn(v)).cloned();
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    let ev = |name: &str| {
                        let n = with_host(|h| h.new_str(name));
                        super::events::instance_call(&target, "emit", vec![n]).map(|_| ())
                    };
                    // Node runs the `end` callbacks, with `null`, before it
                    // emits `finish`.
                    if let Some(cb) = cb {
                        let null = with_host(|h| h.null());
                        invoke(&cb, vec![null], None)?;
                    }
                    ev("finish")?;
                    if auto_close {
                        ev("close")?;
                    }
                    Ok(())
                }))
            });
            Ok(recv.clone())
        }

        "destroy" | "close" => {
            if let Some(fd) = get_prop(recv, "@@wfd").map(|v| with_host(|h| h.to_number(&v)) as i32)
            {
                close_fd(fd);
            }
            let _ =
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))]);
            with_host(|h| h.decr_handle());
            Ok(recv.clone())
        }
        "cork" | "uncork" | "setDefaultEncoding" => Ok(recv.clone()),
        _ => Err(crate::host::type_error(&format!(
            "stream.{method} is not a function"
        ))),
    }
}

fn write_stream_bytes(recv: &Value, chunk: Option<&Value>) {
    let Some(chunk) = chunk else { return };
    let data = value_bytes(chunk);
    if let Some(fd) = get_prop(recv, "@@wfd").map(|v| with_host(|h| h.to_number(&v)) as i32) {
        let written = with_file(fd, |file| {
            let mut fr: &File = file;
            fr.write(&data).unwrap_or(0)
        })
        .unwrap_or(0);
        let prev = get_prop(recv, "bytesWritten")
            .map(|v| with_host(|h| h.to_number(&v)))
            .unwrap_or(0.0);
        set_prop(recv, "bytesWritten", Value::Float(prev + written as f64));
    }
}

// ── Stats / Dirent / Dir instance surfaces ───────────────────────────────────

/// `fs.Stats` method dispatch (`isFile`/`isDirectory`/…).
pub fn stats_call(recv: &Value, method: &str) -> Result<Value, String> {
    type_test(recv, method, "stats")
}

pub const DIRENT_METHODS: &[&str] = &[
    "isFile",
    "isDirectory",
    "isSymbolicLink",
    "isBlockDevice",
    "isCharacterDevice",
    "isFIFO",
    "isSocket",
];

/// `fs.Dirent` method dispatch (tag `Dirent`); `name`/`parentPath`/`path` are
/// plain data properties resolved by the generic object path.
pub fn dirent_call(recv: &Value, method: &str) -> Result<Value, String> {
    type_test(recv, method, "dirent")
}

/// Shared `is*` predicates for `Stats` and `Dirent` (both carry `@@isFile`/
/// `@@isDir`/`@@isSymlink`; the device/FIFO/socket predicates are always false —
/// node-js does not model those file types).
fn type_test(recv: &Value, method: &str, what: &str) -> Result<Value, String> {
    let read = |key: &str| {
        with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => matches!(p.get(key), Some(Value::Bool(true))),
            _ => false,
        })
    };
    match method {
        "isFile" => Ok(Value::Bool(read("@@isFile"))),
        "isDirectory" => Ok(Value::Bool(read("@@isDir"))),
        "isSymbolicLink" => Ok(Value::Bool(read("@@isSymlink"))),
        "isBlockDevice" | "isCharacterDevice" | "isFIFO" | "isSocket" => Ok(Value::Bool(false)),
        _ => Err(crate::host::type_error(&format!(
            "{what}.{method} is not a function"
        ))),
    }
}

pub const DIR_METHODS: &[&str] = &["read", "readSync", "close", "closeSync"];

/// `fs.Dir` method dispatch (tag `Dir`); `path` is a plain data property.
pub fn dir_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        "readSync" => Ok(dir_next(recv)),
        "read" => {
            let v = dir_next(recv);
            if let Some(cb) = args.first().filter(|c| is_fn(c)).cloned() {
                with_host(|h| {
                    let n = h.null();
                    h.queue_io_callback(cb, vec![n, v]);
                });
                Ok(Value::Undef)
            } else {
                Ok(settled_ok(v))
            }
        }
        "closeSync" => Ok(Value::Undef),
        "close" => {
            if let Some(cb) = args.first().filter(|c| is_fn(c)).cloned() {
                with_host(|h| {
                    let n = h.null();
                    h.queue_io_callback(cb, vec![n]);
                });
                Ok(Value::Undef)
            } else {
                Ok(settled_ok(Value::Undef))
            }
        }
        _ => Err(crate::host::type_error(&format!(
            "dir.{method} is not a function"
        ))),
    }
}

/// Advance the `Dir` cursor and return the next `Dirent`, or `null` when drained.
fn dir_next(recv: &Value) -> Value {
    with_host(|h| {
        let (entries, pos) = match h.get(recv) {
            Some(JsObj::Object(p)) => (
                p.get("@@entries").cloned(),
                p.get("@@pos").map(|v| h.to_number(v) as usize).unwrap_or(0),
            ),
            _ => (None, 0),
        };
        let item = match entries.as_ref().and_then(|e| h.get(e)) {
            Some(JsObj::Array(items)) => items.get(pos).cloned(),
            _ => None,
        };
        if item.is_some() {
            if let Some(JsObj::Object(p)) = h.get_mut(recv) {
                p.insert("@@pos".into(), Value::Float((pos + 1) as f64));
            }
        }
        item.unwrap_or_else(|| h.null())
    })
}

fn build_dirent(
    h: &mut crate::host::JsHost,
    name: String,
    parent: &str,
    ft: std::fs::FileType,
) -> Value {
    let mut m = IndexMap::new();
    m.insert("@@native".into(), h.new_str("Dirent"));
    m.insert("name".into(), h.new_str(name));
    let pp = h.new_str(parent.to_string());
    m.insert("parentPath".into(), pp.clone());
    m.insert("path".into(), pp);
    m.insert("@@isFile".into(), Value::Bool(ft.is_file()));
    m.insert("@@isDir".into(), Value::Bool(ft.is_dir()));
    m.insert("@@isSymlink".into(), Value::Bool(ft.is_symlink()));
    h.new_object(m)
}

// ── glob ─────────────────────────────────────────────────────────────────────

fn glob_impl(args: &[Value]) -> Result<Value, String> {
    let pattern = arg_str(args, 0);
    let absolute = pattern.starts_with('/');
    let base = if absolute {
        PathBuf::from("/")
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    let segs: Vec<String> = pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let mut out: Vec<String> = Vec::new();
    let prefix = if absolute {
        "/".to_string()
    } else {
        String::new()
    };
    glob_walk(&base, &segs, 0, &prefix, &mut out);
    out.sort();
    out.dedup();
    Ok(with_host(|h| {
        let items: Vec<Value> = out.into_iter().map(|s| h.new_str(s)).collect();
        h.new_array(items)
    }))
}

fn glob_walk(dir: &Path, segs: &[String], idx: usize, prefix: &str, out: &mut Vec<String>) {
    if idx >= segs.len() {
        if !prefix.is_empty() && prefix != "/" {
            out.push(prefix.trim_end_matches('/').to_string());
        }
        return;
    }
    let seg = &segs[idx];
    if seg == "**" {
        glob_walk(dir, segs, idx + 1, prefix, out);
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let np = join_glob(prefix, &name);
                    glob_walk(&e.path(), segs, idx, &np, out);
                }
            }
        }
        return;
    }
    let last = idx + 1 == segs.len();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !seg.starts_with('.') {
                continue;
            }
            if wildcard_match(seg, &name) {
                let np = join_glob(prefix, &name);
                if last {
                    out.push(np);
                } else if e.path().is_dir() {
                    glob_walk(&e.path(), segs, idx + 1, &np, out);
                }
            }
        }
    }
}

fn join_glob(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else if prefix.ends_with('/') {
        format!("{prefix}{name}")
    } else {
        format!("{prefix}/{name}")
    }
}

/// Match a single path segment against a `*`/`?` glob (no character classes).
fn wildcard_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ── small helpers ────────────────────────────────────────────────────────────

fn settled_ok(v: Value) -> Value {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap_or(0));
    crate::host::resolve_promise_val(id, v);
    p
}

fn get_prop(recv: &Value, key: &str) -> Option<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get(key).cloned(),
        _ => None,
    })
}

fn set_prop(recv: &Value, key: &str, val: Value) {
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert(key.to_string(), val);
        }
    });
}

/// Raw bytes of a byte-view arg, or the utf-8 bytes of anything else.
fn value_bytes(v: &Value) -> Vec<u8> {
    match super::buffer::view_bytes(v) {
        Some(b) => b,
        None => with_host(|h| h.str_of(v)).into_bytes(),
    }
}

fn buf_bytes(v: &Value) -> Vec<u8> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
            Some(JsObj::Array(items)) => items.iter().map(|x| h.to_number(x) as u8).collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

fn array_items(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(v) => with_host(|h| match h.get(v) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        }),
        None => Vec::new(),
    }
}

fn opt_flag(args: &[Value], key: &str) -> bool {
    with_host(|h| {
        args.iter().any(|v| {
            matches!(h.get(v), Some(JsObj::Object(p)) if matches!(p.get(key), Some(Value::Bool(true))))
        })
    })
}

fn encoding_flag(args: &[Value], key: &str) -> Option<String> {
    with_host(|h| {
        for v in args {
            if let Some(JsObj::Object(p)) = h.get(v) {
                if let Some(val) = p.get(key) {
                    return Some(h.str_of(val));
                }
            }
        }
        None
    })
}

fn interval_opt(args: &[Value]) -> Option<f64> {
    with_host(|h| {
        for v in args {
            if let Some(JsObj::Object(p)) = h.get(v) {
                if let Some(val) = p.get("interval") {
                    return Some(h.to_number(val));
                }
            }
        }
        None
    })
}

fn num_or(args: &[Value], i: usize, default: f64) -> f64 {
    match args.get(i) {
        Some(v) if !matches!(v, Value::Undef) => {
            let n = with_host(|h| h.to_number(v));
            if n.is_nan() {
                default
            } else {
                n
            }
        }
        _ => default,
    }
}

/// A byte `position` argument: `null`/`undefined` means "current file offset".
fn position_arg(args: &[Value], i: usize) -> Option<u64> {
    match args.get(i) {
        Some(Value::Undef) | None => None,
        Some(v) if with_host(|h| h.is_null(v)) => None,
        Some(v) => {
            let n = with_host(|h| h.to_number(v));
            if n.is_nan() || n < 0.0 {
                None
            } else {
                Some(n as u64)
            }
        }
    }
}

/// A timestamp argument in **seconds** (`toUnixTimestamp`). `validate_call`
/// has already refused one that does not convert.
fn time_secs(args: &[Value], i: usize) -> f64 {
    to_unix_timestamp(args.get(i).unwrap_or(&Value::Undef), "time").unwrap_or(f64::NAN)
}

fn to_timeval(secs: f64) -> libc::timeval {
    let s = secs.floor();
    let us = ((secs - s) * 1_000_000.0).round();
    libc::timeval {
        tv_sec: s as libc::time_t,
        tv_usec: us as libc::suseconds_t,
    }
}

fn cpath(path: &str, op: &str) -> Result<CString, String> {
    CString::new(path).map_err(|_| format!("Error: EINVAL: invalid argument, {op} '{path}'"))
}

fn ok_or_errno(rc: libc::c_int, op: &str, path: &str) -> Result<Value, String> {
    if rc == 0 {
        Ok(Value::Undef)
    } else {
        Err(err_str(op, path, &std::io::Error::last_os_error()))
    }
}

/// The encoding an fs call was given, from either the string form
/// (`readFileSync(p, 'hex')`) or the options-object form
/// (`readFileSync(p, { encoding: 'hex' })`).
///
/// The object form used to be discarded — it was detected by its
/// `"[object Object]"` stringification and treated as absent — so
/// `{ encoding: 'utf8' }` returned a Buffer. `{ encoding: null }` really does
/// mean "no encoding", and so returns `None`.
fn encoding_arg(args: &[Value], i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::Undef) | None => None,
        Some(v) => {
            if with_host(|h| matches!(h.get(v), Some(JsObj::Object(_)))) {
                let e = crate::builtins::get_property(v, "encoding").ok()?;
                if matches!(e, Value::Undef) || with_host(|h| h.is_null(&e)) {
                    return None;
                }
                return Some(with_host(|h| h.str_of(&e)));
            }
            let s = with_host(|h| h.str_of(v));
            if s == "undefined" || s == "null" {
                None
            } else {
                Some(s)
            }
        }
    }
}

/// The failing *syscall* Node names in a filesystem error, for the JS-level
/// entry points whose API name differs from it. `fs.readFileSync('/nope')`
/// reports `open`, not `readFileSync`, because `open(2)` is what failed —
/// and `err.syscall` is load-bearing for packages that branch on it.
fn syscall_name(op: &str) -> &str {
    match op {
        "readFileSync" | "readFile" | "writeFile" | "writeFileSync" | "appendFile"
        | "appendFileSync" | "createReadStream" | "createWriteStream" => "open",
        "readdir" | "readdirSync" | "opendir" => "scandir",
        // The rest name the C call, which for most of these is the JS name
        // minus its `Sync` suffix — `fs.statSync('/nope')` reports `stat`. Only
        // the ones whose spelling ALSO changes need an entry: libuv writes
        // `copyfile` in one word, and `realpath` fails inside the `lstat` it
        // walks the path with.
        "copyFile" | "copyFileSync" => "copyfile",
        "realpath" | "realpathSync" => "lstat",
        other => other.strip_suffix("Sync").unwrap_or(other),
    }
}

/// The second path a two-argument call names in its message: node renders
/// `rename '/a' -> '/b'`, and a message with only the source reads as though
/// the destination were never part of the operation.
fn err_str2(op: &str, from: &str, to: &str, e: &std::io::Error) -> String {
    format!("{} -> '{to}'", err_str(op, from, e))
}

/// The `CODE: reason` half of a libuv error message (`ENOENT: no such file or
/// directory`), shared by every syscall wrapper that has to render one.
///
/// Deliberately NOT Rust's `io::Error` `Display`: Rust capitalizes and appends
/// its own `(os error 2)`, so `No such file or directory (os error 2)` is a
/// string no Node has ever printed. `process.chdir` was assembling its message
/// that way.
/// The libuv error CODE for an `io::Error` — `ENOENT`, `EACCES` and the rest.
/// Node exposes it as `err.code`, and every error it renders leads with it.
pub(crate) fn libuv_code(e: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    // An OS error names itself: its errno is looked up in the same table
    // `util.getSystemErrorMap` reports, so `EBADF`, `ELOOP`, `ENAMETOOLONG`,
    // `EXDEV`, … keep their own codes. Mapping by `ErrorKind` alone folded
    // every kind Rust does not single out into `EIO`.
    if let Some(name) = e.raw_os_error().and_then(crate::stdlib::util::errno_name) {
        return name;
    }
    match e.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        NotADirectory => "ENOTDIR",
        IsADirectory => "EISDIR",
        DirectoryNotEmpty => "ENOTEMPTY",
        InvalidInput => "EINVAL",
        BrokenPipe => "EPIPE",
        _ => "EIO",
    }
}

pub(crate) fn libuv_message(e: &std::io::Error) -> String {
    let code = libuv_code(e);
    format!("{code}: {}", libuv_reason(code))
}

/// libuv's own lowercase reason string for a code.
fn libuv_reason(code: &str) -> &'static str {
    crate::stdlib::util::errno_by_name(code).map_or("i/o error", |(_, m)| m)
}

fn err_str(op: &str, path: &str, e: &std::io::Error) -> String {
    let code = libuv_code(e);
    // libuv's own reason strings, not Rust's `io::Error` `Display` — Rust
    // capitalizes ("No such file or directory") where libuv (and therefore
    // Node's `err.message`) does not.
    let reason = libuv_reason(code);
    let op = syscall_name(op);
    if path.is_empty() {
        format!("Error: {code}: {reason}, {op}")
    } else {
        format!("Error: {code}: {reason}, {op} '{path}'")
    }
}
