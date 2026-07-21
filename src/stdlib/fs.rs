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
use std::io::{Read, Seek, SeekFrom, Write};
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

fn close_fd(fd: i32) -> bool {
    FD_TABLE.with(|t| t.borrow_mut().remove(&fd).is_some())
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
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
        "read" => return Some(read_write_async(args, read_impl(args))),
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
            h.queue_micro(cb, vec![n, v]);
        }),
        Err(e) => with_host(|h| {
            let ev = h.new_str(e);
            h.queue_micro(cb, vec![ev]);
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
            h.queue_micro(cb, vec![nul, Value::Float(n as f64), buffer]);
        }),
        Err(e) => with_host(|h| {
            let ev = h.new_str(e);
            h.queue_micro(cb, vec![ev]);
        }),
    }
    Ok(Value::Undef)
}

fn is_fn(v: &Value) -> bool {
    with_host(|h| crate::host::is_callable(h, v))
}

// ── read/write/append ────────────────────────────────────────────────────────

fn read_file_sync(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let enc = encoding_arg(args, 1);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(match enc {
            Some(_) => with_host(|h| h.new_str(String::from_utf8_lossy(&bytes).into_owned())),
            None => super::buffer::from_bytes(&bytes),
        }),
        Err(e) => Err(err_str("readFileSync", &path, &e)),
    }
}

fn write_file_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let data = value_bytes(args.get(1).unwrap_or(&Value::Undef));
    match std::fs::write(&path, data) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("writeFile", &path, &e)),
    }
}

fn append_file_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let data = value_bytes(args.get(1).unwrap_or(&Value::Undef));
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
            match enc {
                Some(_) => with_host(|h| h.new_str(String::from_utf8_lossy(&bytes).into_owned())),
                None => super::buffer::from_bytes(&bytes),
            },
        ),
        Err(e) => (
            with_host(|h| h.new_str(err_str("readFile", &path, &e))),
            Value::Undef,
        ),
    };
    with_host(|h| h.queue_micro(cb, vec![err, data]));
    Ok(Value::Undef)
}

fn exists_async(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let Some(cb) = args.last().cloned().filter(is_fn) else {
        return Ok(Value::Undef);
    };
    let ex = Path::new(&path).exists();
    with_host(|h| h.queue_micro(cb, vec![Value::Bool(ex)]));
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
    let mode = arg_num(args, 1) as u32;
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
        Err(e) => Err(err_str("copyFile", &src, &e)),
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
        Err(e) => Err(err_str("realpath", &path, &e)),
    }
}

fn rename_impl(args: &[Value]) -> Result<Value, String> {
    let from = arg_str(args, 0);
    let to = arg_str(args, 1);
    match std::fs::rename(&from, &to) {
        Ok(_) => Ok(Value::Undef),
        Err(e) => Err(err_str("rename", &from, &e)),
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
        Err(e) => Err(err_str("truncate", &path, &e)),
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

// ── file descriptors ─────────────────────────────────────────────────────────

fn open_impl(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let flags = match args.get(1) {
        Some(v) if !matches!(v, Value::Undef) && !is_fn(v) => arg_str(args, 1),
        _ => "r".to_string(),
    };
    match open_options(&flags).open(&path) {
        Ok(f) => Ok(Value::Float(register_fd(f) as f64)),
        Err(e) => Err(err_str("open", &path, &e)),
    }
}

fn open_options(flags: &str) -> std::fs::OpenOptions {
    let mut o = std::fs::OpenOptions::new();
    match flags {
        "r" | "rs" | "sr" => {
            o.read(true);
        }
        "r+" | "rs+" | "sr+" => {
            o.read(true).write(true);
        }
        "w" => {
            o.write(true).create(true).truncate(true);
        }
        "wx" | "xw" => {
            o.write(true).create_new(true);
        }
        "w+" => {
            o.read(true).write(true).create(true).truncate(true);
        }
        "wx+" | "xw+" => {
            o.read(true).write(true).create_new(true);
        }
        "a" => {
            o.append(true).create(true);
        }
        "ax" | "xa" => {
            o.append(true).create_new(true);
        }
        "a+" => {
            o.read(true).append(true).create(true);
        }
        "ax+" | "xa+" => {
            o.read(true).append(true).create_new(true);
        }
        _ => {
            o.read(true);
        }
    }
    o
}

fn close_impl(args: &[Value]) -> Result<Value, String> {
    let fd = arg_num(args, 0) as i32;
    if close_fd(fd) {
        Ok(Value::Undef)
    } else {
        Err("Error: EBADF: bad file descriptor, close".to_string())
    }
}

fn read_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let buffer = args.get(1).cloned().unwrap_or(Value::Undef);
    let cap = buf_len(&buffer);
    let offset = num_or(args, 2, 0.0) as usize;
    let length = num_or(args, 3, (cap.saturating_sub(offset)) as f64) as usize;
    let position = position_arg(args, 4);
    let n = with_file(fd, |file| {
        let mut fr: &File = file;
        if let Some(pos) = position {
            fr.seek(SeekFrom::Start(pos)).ok();
        }
        let mut buf = vec![0u8; length];
        fr.read(&mut buf).map(|n| {
            buf.truncate(n);
            buf
        })
    });
    match n {
        Some(Ok(data)) => Ok(write_into_buffer(&buffer, offset, &data)),
        Some(Err(e)) => Err(err_str("read", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, read".to_string()),
    }
}

fn write_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let src = args.get(1).cloned().unwrap_or(Value::Undef);
    let is_buffer = native_tag(&src).as_deref() == Some("Buffer");
    // Buffer form: write(fd, buffer, offset, length, position)
    // String form: write(fd, string, position, encoding)
    let (data, position) = if is_buffer {
        let all = buf_bytes(&src);
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
    let r = with_file(fd, |file| {
        let mut fr: &File = file;
        if let Some(pos) = position {
            fr.seek(SeekFrom::Start(pos)).ok();
        }
        fr.write(&data)
    });
    match r {
        Some(Ok(n)) => Ok(n),
        Some(Err(e)) => Err(err_str("write", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, write".to_string()),
    }
}

fn readv_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let buffers = array_items(args.get(1));
    let position = position_arg(args, 2);
    let total = with_file(fd, |file| {
        let mut fr: &File = file;
        if let Some(pos) = position {
            fr.seek(SeekFrom::Start(pos)).ok();
        }
        let mut chunks: Vec<(Value, Vec<u8>)> = Vec::new();
        for b in &buffers {
            let cap = buf_len(b);
            let mut buf = vec![0u8; cap];
            match fr.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    chunks.push((b.clone(), buf));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(chunks)
    });
    match total {
        Some(Ok(chunks)) => Ok(chunks.iter().map(|(b, d)| write_into_buffer(b, 0, d)).sum()),
        Some(Err(e)) => Err(err_str("readv", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, readv".to_string()),
    }
}

fn writev_impl(args: &[Value]) -> Result<usize, String> {
    let fd = arg_num(args, 0) as i32;
    let buffers = array_items(args.get(1));
    let position = position_arg(args, 2);
    let mut data = Vec::new();
    for b in &buffers {
        data.extend(buf_bytes(b));
    }
    let r = with_file(fd, |file| {
        let mut fr: &File = file;
        if let Some(pos) = position {
            fr.seek(SeekFrom::Start(pos)).ok();
        }
        fr.write(&data)
    });
    match r {
        Some(Ok(n)) => Ok(n),
        Some(Err(e)) => Err(err_str("writev", "", &e)),
        None => Err("Error: EBADF: bad file descriptor, writev".to_string()),
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
    let mode = arg_num(args, 1) as libc::mode_t;
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
    h.new_object(m)
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
    let recv = stream.clone();
    let p = path;
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            read_stream_pump(&recv, &p);
            Ok(())
        }))
    });
    Ok(stream)
}

/// Read the whole file and drive the stream's events on the microtask tick after
/// creation (so synchronously-attached `on('data')`/`on('end')` listeners fire).
fn read_stream_pump(recv: &Value, path: &str) {
    with_host(|h| h.decr_handle());
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
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
    let enc = get_prop(recv, "@@encoding").map(|v| with_host(|h| h.str_of(&v)));
    let chunk = match enc.as_deref() {
        Some(e) if e != "buffer" => {
            with_host(|h| h.new_str(String::from_utf8_lossy(&bytes).into_owned()))
        }
        _ => super::buffer::from_bytes(&bytes),
    };
    if let Some(dest) = get_prop(recv, "@@pipeDest") {
        let _ = crate::host::call_method(&dest, "write", vec![chunk]);
        let _ = crate::host::call_method(&dest, "end", vec![]);
    } else {
        let name = with_host(|h| h.new_str("data"));
        let _ = super::events::instance_call(recv, "emit", vec![name, chunk]);
    }
    let _ = super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("end"))]);
    let _ = super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))]);
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

/// `fs.ReadStream` instance dispatch (tag `FSReadStream`). Emitter methods are
/// handled by the parent's shared emitter routing; this covers the stream-only
/// surface.
pub fn read_stream_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
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
    let file = open_options(&flags)
        .open(&path)
        .map_err(|e| err_str("open", &path, &e))?;
    let fd = register_fd(file);
    let stream = with_host(|h| {
        let mut extra = IndexMap::new();
        extra.insert("path".into(), h.new_str(path));
        extra.insert("@@wfd".into(), Value::Float(fd as f64));
        extra.insert("bytesWritten".into(), Value::Float(0.0));
        extra
    });
    let stream = super::net::new_emitter_object("FSWriteStream", stream);
    with_host(|h| h.incr_handle());
    let recv = stream.clone();
    with_host(|h| {
        h.queue_micro_native(Box::new(move || {
            let _ =
                super::events::instance_call(&recv, "emit", vec![with_host(|h| h.new_str("open"))]);
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

/// `fs.WriteStream` instance dispatch (tag `FSWriteStream`).
pub fn write_stream_call(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match method {
        "write" => {
            write_stream_bytes(recv, args.first());
            if let Some(cb) = args.iter().find(|v| is_fn(v)).cloned() {
                let _ = invoke(&cb, vec![], None);
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
            if let Some(fd) = get_prop(recv, "@@wfd").map(|v| with_host(|h| h.to_number(&v)) as i32)
            {
                close_fd(fd);
            }
            let _ = super::events::instance_call(
                recv,
                "emit",
                vec![with_host(|h| h.new_str("finish"))],
            );
            let _ =
                super::events::instance_call(recv, "emit", vec![with_host(|h| h.new_str("close"))]);
            with_host(|h| h.decr_handle());
            if let Some(cb) = args.iter().find(|v| is_fn(v)).cloned() {
                let _ = invoke(&cb, vec![], None);
            }
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
                    h.queue_micro(cb, vec![n, v]);
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
                    h.queue_micro(cb, vec![n]);
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

/// Raw bytes of a Buffer arg, or the utf-8 bytes of anything else.
fn value_bytes(v: &Value) -> Vec<u8> {
    if native_tag(v).as_deref() == Some("Buffer") {
        buf_bytes(v)
    } else {
        with_host(|h| h.str_of(v)).into_bytes()
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

fn buf_len(v: &Value) -> usize {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get("@@bytes").and_then(|b| h.get(b)) {
            Some(JsObj::Array(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    })
}

/// Write `data` into a Buffer's backing byte array at `offset`; returns the count
/// actually written (bounded by the buffer capacity).
fn write_into_buffer(buf: &Value, offset: usize, data: &[u8]) -> usize {
    let Some(arr) = get_prop(buf, "@@bytes") else {
        return 0;
    };
    with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(&arr) {
            let mut n = 0;
            for (i, b) in data.iter().enumerate() {
                let idx = offset + i;
                if idx >= items.len() {
                    break;
                }
                items[idx] = Value::Float(*b as f64);
                n += 1;
            }
            n
        } else {
            0
        }
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

/// A timestamp argument in **seconds**. A `Date` argument stores milliseconds, so
/// divide; a bare number is already seconds (Node's `fs.utimes` convention).
fn time_secs(args: &[Value], i: usize) -> f64 {
    match args.get(i) {
        Some(v) if native_tag(v).as_deref() == Some("Date") => arg_num(args, i) / 1000.0,
        _ => arg_num(args, i),
    }
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

fn encoding_arg(args: &[Value], i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::Undef) | None => None,
        Some(v) => {
            let s = with_host(|h| h.str_of(v));
            if s == "undefined" || s == "[object Object]" || s == "null" {
                None
            } else {
                Some(s)
            }
        }
    }
}

fn err_str(op: &str, path: &str, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    let code = match e.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        _ => "EIO",
    };
    let reason = e.to_string();
    let reason = reason.split(" (os error").next().unwrap_or("error");
    if path.is_empty() {
        format!("Error: {code}: {reason}, {op}")
    } else {
        format!("Error: {code}: {reason}, {op} '{path}'")
    }
}
