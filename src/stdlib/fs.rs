//! Node `fs` module: synchronous file operations plus the basic async
//! callback forms (`readFile`/`writeFile`, whose callbacks are scheduled as
//! microtasks so they run after the current tick, matching Node's ordering for
//! simple scripts).

use super::arg_str;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
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
    "readFile",
    "writeFile",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "readFileSync" => read_file_sync(args),
        "writeFileSync" => {
            let path = arg_str(args, 0);
            let data = arg_str(args, 1);
            match std::fs::write(&path, data) {
                Ok(_) => Ok(Value::Undef),
                Err(e) => Err(err_str("writeFileSync", &path, &e)),
            }
        }
        "appendFileSync" => {
            use std::io::Write;
            let path = arg_str(args, 0);
            let data = arg_str(args, 1);
            let r = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(data.as_bytes()));
            match r {
                Ok(_) => Ok(Value::Undef),
                Err(e) => Err(err_str("appendFileSync", &path, &e)),
            }
        }
        "existsSync" => Ok(Value::Bool(std::path::Path::new(&arg_str(args, 0)).exists())),
        "readdirSync" => read_dir_sync(args),
        "mkdirSync" => {
            let path = arg_str(args, 0);
            let recursive = args.get(1).map(is_recursive).unwrap_or(false);
            let r = if recursive { std::fs::create_dir_all(&path) } else { std::fs::create_dir(&path) };
            match r {
                Ok(_) => Ok(Value::Undef),
                Err(e) => Err(err_str("mkdirSync", &path, &e)),
            }
        }
        "rmdirSync" | "unlinkSync" | "rmSync" => {
            let path = arg_str(args, 0);
            let p = std::path::Path::new(&path);
            let r = if p.is_dir() {
                if args.get(1).map(is_recursive).unwrap_or(false) { std::fs::remove_dir_all(p) } else { std::fs::remove_dir(p) }
            } else {
                std::fs::remove_file(p)
            };
            match r {
                Ok(_) => Ok(Value::Undef),
                Err(e) => Err(err_str(method, &path, &e)),
            }
        }
        "statSync" => stat_sync(args),
        "readFile" => read_file_async(args),
        "writeFile" => write_file_async(args),
        _ => return None,
    })
}

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

fn read_dir_sync(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::read_dir(&path) {
        Ok(rd) => {
            let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            Ok(with_host(|h| {
                let items: Vec<Value> = names.into_iter().map(|n| h.new_str(n)).collect();
                h.new_array(items)
            }))
        }
        Err(e) => Err(err_str("readdirSync", &path, &e)),
    }
}

fn stat_sync(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    match std::fs::metadata(&path) {
        Ok(md) => Ok(with_host(|h| {
            let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
            let mut m = IndexMap::new();
            m.insert("@@native".into(), h.new_str("Stats"));
            m.insert("@@isFile".into(), Value::Bool(md.is_file()));
            m.insert("@@isDir".into(), Value::Bool(md.is_dir()));
            m.insert("@@isSymlink".into(), Value::Bool(md.file_type().is_symlink()));
            m.insert("size".into(), Value::Float(md.len() as f64));
            m.insert("mtimeMs".into(), Value::Float(mtime));
            h.new_object(m)
        })),
        Err(e) => Err(err_str("statSync", &path, &e)),
    }
}

/// `fs.Stats` method dispatch (`isFile`/`isDirectory`/`isSymbolicLink`).
pub fn stats_call(recv: &Value, method: &str) -> Result<Value, String> {
    let read = |key: &str| with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => matches!(p.get(key), Some(Value::Bool(true))),
        _ => false,
    });
    match method {
        "isFile" => Ok(Value::Bool(read("@@isFile"))),
        "isDirectory" => Ok(Value::Bool(read("@@isDir"))),
        "isSymbolicLink" => Ok(Value::Bool(read("@@isSymlink"))),
        "isBlockDevice" | "isCharacterDevice" | "isFIFO" | "isSocket" => Ok(Value::Bool(false)),
        _ => Err(crate::host::type_error(&format!("stats.{method} is not a function"))),
    }
}

fn read_file_async(args: &[Value]) -> Result<Value, String> {
    // Last argument is the callback; an optional middle arg is the encoding.
    let path = arg_str(args, 0);
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    let enc = if args.len() >= 3 { encoding_arg(args, 1) } else { None };
    let (err, data) = match std::fs::read(&path) {
        Ok(bytes) => (
            with_host(|h| h.null()),
            match enc {
                Some(_) => with_host(|h| h.new_str(String::from_utf8_lossy(&bytes).into_owned())),
                None => super::buffer::from_bytes(&bytes),
            },
        ),
        Err(e) => (with_host(|h| h.new_str(err_str("readFile", &path, &e))), Value::Undef),
    };
    with_host(|h| h.queue_micro(cb, vec![err, data]));
    Ok(Value::Undef)
}

fn write_file_async(args: &[Value]) -> Result<Value, String> {
    let path = arg_str(args, 0);
    let data = arg_str(args, 1);
    let Some(cb) = args.last().cloned() else { return Ok(Value::Undef) };
    let err = match std::fs::write(&path, data) {
        Ok(_) => with_host(|h| h.null()),
        Err(e) => with_host(|h| h.new_str(err_str("writeFile", &path, &e))),
    };
    with_host(|h| h.queue_micro(cb, vec![err]));
    Ok(Value::Undef)
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

fn is_recursive(v: &Value) -> bool {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => matches!(p.get("recursive"), Some(Value::Bool(true))),
        _ => false,
    })
}

fn err_str(op: &str, path: &str, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    let code = match e.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        _ => "EIO",
    };
    format!("Error: {code}: {}, {op} '{path}'", e.to_string().split(" (os error").next().unwrap_or("error"))
}
