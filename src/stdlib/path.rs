//! Node `path` module (POSIX semantics, matching macOS/Linux `path`).

use super::arg_str;
use crate::host::with_host;
use fusevm::Value;
use indexmap::IndexMap;

pub const METHODS: &[&str] = &[
    "join",
    "resolve",
    "normalize",
    "basename",
    "dirname",
    "extname",
    "isAbsolute",
    "relative",
    "parse",
    "format",
];

/// `path.sep` / `path.delimiter` constants.
pub fn constant(name: &str) -> Option<Value> {
    match name {
        "sep" => Some(with_host(|h| h.new_str("/"))),
        "delimiter" => Some(with_host(|h| h.new_str(":"))),
        _ => None,
    }
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let parts: Vec<String> = (0..args.len()).map(|i| arg_str(args, i)).collect();
    let s = |v: String| Ok(with_host(|h| h.new_str(v)));
    Some(match method {
        "join" => s(join(&parts)),
        "resolve" => s(resolve(&parts)),
        "normalize" => s(normalize(&first(&parts))),
        "basename" => s(basename(&first(&parts), parts.get(1).map(|x| x.as_str()))),
        "dirname" => s(dirname(&first(&parts))),
        "extname" => s(extname(&first(&parts))),
        "isAbsolute" => Ok(Value::Bool(first(&parts).starts_with('/'))),
        "relative" => s(relative(&first(&parts), parts.get(1).cloned().unwrap_or_default().as_str())),
        "parse" => Ok(parse(&first(&parts))),
        "format" => s(format(args.first())),
        _ => return None,
    })
}

fn first(parts: &[String]) -> String {
    parts.first().cloned().unwrap_or_default()
}

fn join(parts: &[String]) -> String {
    let joined: Vec<&str> = parts.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
    if joined.is_empty() {
        return ".".into();
    }
    normalize(&joined.join("/"))
}

/// Collapse `.`/`..` segments and duplicate slashes, preserving a leading `/` and
/// a trailing `/` — Node's `path.normalize` behaviour.
fn normalize(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let is_abs = p.starts_with('/');
    let trailing = p.ends_with('/') && p.len() > 1;
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if let Some(&last) = out.last() {
                    if last != ".." {
                        out.pop();
                        continue;
                    }
                }
                if !is_abs {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let mut body = out.join("/");
    if body.is_empty() {
        return if is_abs { "/".into() } else { ".".into() };
    }
    if is_abs {
        body.insert(0, '/');
    }
    if trailing {
        body.push('/');
    }
    body
}

fn basename(p: &str, ext: Option<&str>) -> String {
    let base = p.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    match ext {
        Some(e) if base.ends_with(e) && base != *e => base[..base.len() - e.len()].to_string(),
        _ => base,
    }
}

fn dirname(p: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].to_string(),
        None => ".".into(),
    }
}

fn extname(p: &str) -> String {
    let base = basename(p, None);
    match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_string(),
        _ => String::new(),
    }
}

fn resolve(parts: &[String]) -> String {
    let mut resolved = String::new();
    let mut is_abs = false;
    for p in parts.iter().rev() {
        if p.is_empty() {
            continue;
        }
        resolved = if resolved.is_empty() { p.clone() } else { format!("{p}/{resolved}") };
        if p.starts_with('/') {
            is_abs = true;
            break;
        }
    }
    if !is_abs {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
        resolved = if resolved.is_empty() { cwd } else { format!("{cwd}/{resolved}") };
    }
    let n = normalize(&resolved);
    // resolve never keeps a trailing slash (except root).
    if n.len() > 1 { n.trim_end_matches('/').to_string() } else { n }
}

fn relative(from: &str, to: &str) -> String {
    let from = resolve(&[from.to_string()]);
    let to = resolve(&[to.to_string()]);
    let fs: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let ts: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = fs.iter().zip(ts.iter()).take_while(|(a, b)| a == b).count();
    let mut out: Vec<String> = vec!["..".into(); fs.len() - common];
    out.extend(ts[common..].iter().map(|s| s.to_string()));
    if out.is_empty() { String::new() } else { out.join("/") }
}

fn parse(p: &str) -> Value {
    let root = if p.starts_with('/') { "/" } else { "" };
    let dir = dirname(p);
    let base = basename(p, None);
    let ext = extname(p);
    let name = base.strip_suffix(&ext).unwrap_or(&base).to_string();
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("root".into(), h.new_str(root));
        m.insert("dir".into(), h.new_str(dir));
        m.insert("base".into(), h.new_str(base));
        m.insert("ext".into(), h.new_str(ext));
        m.insert("name".into(), h.new_str(name));
        h.new_object(m)
    })
}

fn format(obj: Option<&Value>) -> String {
    let Some(obj) = obj else { return String::new() };
    let get = |k: &str| with_host(|h| match h.get(obj) {
        Some(crate::host::JsObj::Object(p)) => p.get(k).map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    });
    let dir = get("dir");
    let root = get("root");
    let base = if !get("base").is_empty() {
        get("base")
    } else {
        format!("{}{}", get("name"), get("ext"))
    };
    let d = if !dir.is_empty() { dir } else { root };
    if d.is_empty() {
        base
    } else if d.ends_with('/') {
        format!("{d}{base}")
    } else {
        format!("{d}/{base}")
    }
}
