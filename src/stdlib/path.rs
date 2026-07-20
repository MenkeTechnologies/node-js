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
    "matchesGlob",
    "toNamespacedPath",
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
        "matchesGlob" => Ok(Value::Bool(matches_glob(&first(&parts), parts.get(1).map(|x| x.as_str()).unwrap_or("")))),
        // POSIX `toNamespacedPath` is the identity (Windows-only namespacing).
        "toNamespacedPath" => s(first(&parts)),
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

/// `path.matchesGlob(path, pattern)` — whether `path` matches the glob `pattern`.
/// Supports `*` (within a segment), `**` (across `/`), `?`, `[...]` classes, and
/// top-level `{a,b}` brace alternatives — the minimatch-style subset Node uses.
fn matches_glob(path: &str, pattern: &str) -> bool {
    let text: Vec<char> = path.chars().collect();
    expand_braces(pattern)
        .iter()
        .any(|pat| glob_match(&text, &pat.chars().collect::<Vec<char>>()))
}

/// Expand top-level `{a,b,c}` alternatives into concrete pattern strings.
fn expand_braces(pattern: &str) -> Vec<String> {
    let chars: Vec<char> = pattern.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c != '{' {
            continue;
        }
        let mut depth = 1;
        let mut commas: Vec<usize> = Vec::new();
        let mut close = None;
        for (j, &cj) in chars.iter().enumerate().skip(i + 1) {
            match cj {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                ',' if depth == 1 => commas.push(j),
                _ => {}
            }
        }
        let (Some(close), false) = (close, commas.is_empty()) else { continue };
        let prefix: String = chars[..i].iter().collect();
        let suffix: String = chars[close + 1..].iter().collect();
        let mut bounds = vec![i];
        bounds.extend(&commas);
        bounds.push(close);
        let mut out = Vec::new();
        for w in bounds.windows(2) {
            let alt: String = chars[w[0] + 1..w[1]].iter().collect();
            out.extend(expand_braces(&format!("{prefix}{alt}{suffix}")));
        }
        return out;
    }
    vec![pattern.to_string()]
}

/// Recursive glob matcher over char slices. `*` never crosses `/`, `**` does.
fn glob_match(t: &[char], p: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            let double = p.len() >= 2 && p[1] == '*';
            let rest = {
                let mut k = 0;
                while k < p.len() && p[k] == '*' {
                    k += 1;
                }
                &p[k..]
            };
            if rest.is_empty() {
                return double || !t.contains(&'/');
            }
            let mut ti = 0;
            loop {
                if glob_match(&t[ti..], rest) {
                    return true;
                }
                if ti >= t.len() {
                    return false;
                }
                if !double && t[ti] == '/' {
                    return false;
                }
                ti += 1;
            }
        }
        '?' => !t.is_empty() && t[0] != '/' && glob_match(&t[1..], &p[1..]),
        '[' => match match_class(t.first().copied(), p) {
            Some((matched, plen)) => matched && glob_match(&t[1..], &p[plen..]),
            // Unterminated `[` is a literal bracket.
            None => !t.is_empty() && t[0] == '[' && glob_match(&t[1..], &p[1..]),
        },
        c => !t.is_empty() && t[0] == c && glob_match(&t[1..], &p[1..]),
    }
}

/// Match `ch` against a `[...]` class starting at `p[0] == '['`. Returns
/// `(matched, chars_consumed)`, or `None` when the class is unterminated.
fn match_class(ch: Option<char>, p: &[char]) -> Option<(bool, usize)> {
    let mut i = 1;
    let mut negate = false;
    if matches!(p.get(i), Some('!') | Some('^')) {
        negate = true;
        i += 1;
    }
    let start = i;
    let mut matched = false;
    while i < p.len() && (p[i] != ']' || i == start) {
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            if let Some(c) = ch {
                if p[i] <= c && c <= p[i + 2] {
                    matched = true;
                }
            }
            i += 3;
        } else {
            if ch == Some(p[i]) {
                matched = true;
            }
            i += 1;
        }
    }
    if i >= p.len() {
        return None;
    }
    // `ch` is None (empty text) or a `/` never matches a class.
    let ok = matches!(ch, Some(c) if c != '/') && (matched ^ negate);
    Some((ok, i + 1))
}
