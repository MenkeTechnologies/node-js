//! Node `path` module — both flavors.
//!
//! A faithful port of Node's `lib/path.js` (v26): the POSIX flavor (which is
//! what `require('path')` and `path.posix` yield on a POSIX host) and the
//! Windows flavor (`path.win32` / `require('path/win32')`), sharing the same
//! `normalize_string` core. The algorithms mirror the JS line-for-line —
//! index arithmetic included — so edge cases (UNC roots, device roots, reserved
//! device names, the CVE-2024-36139 relative-drive guard, trailing-separator
//! retention) behave exactly as Node's do rather than approximately.
//!
//! Node scans paths by UTF-16 code unit; this port scans by `char`. Every
//! comparison is against ASCII (`/`, `\`, `.`, `:`, drive letters) and every
//! slice boundary is derived from those comparisons, so the produced strings
//! are identical — only the intermediate index values differ on astral input.

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
    // Legacy internal alias for `toNamespacedPath` (docs-only deprecated DEP0080).
    "_makeLong",
];

/// Which separator flavor a `path` call runs under.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Posix,
    Win32,
}

impl Flavor {
    /// Node's per-flavor `isPathSeparator`: win32 accepts both slashes.
    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == Flavor::Win32 && c == '\\')
    }

    fn sep(self) -> char {
        match self {
            Flavor::Posix => '/',
            Flavor::Win32 => '\\',
        }
    }

    fn delimiter(self) -> &'static str {
        match self {
            Flavor::Posix => ":",
            Flavor::Win32 => ";",
        }
    }
}

/// `path.sep` / `path.delimiter` constants for `flavor`.
pub fn constant(flavor: Flavor, name: &str) -> Option<Value> {
    match name {
        "sep" => Some(with_host(|h| h.new_str(flavor.sep().to_string()))),
        "delimiter" => Some(with_host(|h| h.new_str(flavor.delimiter()))),
        _ => None,
    }
}

pub fn call(flavor: Flavor, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let parts: Vec<String> = (0..args.len()).map(|i| arg_str(args, i)).collect();
    let s = |v: String| Ok(with_host(|h| h.new_str(v)));
    let one = |i: usize| chars(parts.get(i).map(String::as_str).unwrap_or(""));
    Some(match method {
        "join" => s(join(flavor, &parts)),
        "resolve" => s(resolve(flavor, &parts)),
        "normalize" => s(normalize(flavor, &one(0))),
        "basename" => s(basename(
            flavor,
            &one(0),
            // An absent 2nd arg is `undefined`, not `""` — Node skips the
            // suffix-matching branch entirely in that case.
            parts.get(1).map(|x| chars(x)).as_deref(),
        )),
        "dirname" => s(dirname(flavor, &one(0))),
        "extname" => s(extname(flavor, &one(0))),
        "isAbsolute" => Ok(Value::Bool(is_absolute(flavor, &one(0)))),
        "relative" => s(relative(flavor, &one(0), &one(1))),
        "parse" => Ok(parse(flavor, &one(0))),
        "format" => s(format(flavor, args.first())),
        "matchesGlob" => Ok(Value::Bool(matches_glob(
            flavor,
            parts.first().map(String::as_str).unwrap_or(""),
            parts.get(1).map(|x| x.as_str()).unwrap_or(""),
        ))),
        "toNamespacedPath" | "_makeLong" => s(to_namespaced_path(
            flavor,
            parts.first().map(String::as_str).unwrap_or(""),
        )),
        _ => return None,
    })
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn str_of(c: &[char]) -> String {
    c.iter().collect()
}

/// Node's `isWindowsDeviceRoot`: an ASCII letter usable as a drive letter.
fn is_device_root(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// Device names Windows reserves regardless of directory (`CON`, `LPT3`, …).
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

/// Node's `isWindowsReservedName(path, colonIndex)` — is `path.slice(0,
/// colonIndex)` a reserved device name? `colon_index` is an `indexOf` result and
/// may be negative, which in JS counts back from the end (`"CON/".slice(0, -1)`
/// is `"CON"`), so a colon-less `"CON/"` IS reserved. Reproducing that is what
/// makes `path.win32.normalize("CON/")` return `.\CON\` rather than `CON\`.
fn is_reserved_name(p: &[char], colon_index: isize) -> bool {
    let end = if colon_index < 0 {
        (p.len() as isize + colon_index).max(0) as usize
    } else {
        (colon_index as usize).min(p.len())
    };
    let device: String = p[..end].iter().collect::<String>().to_uppercase();
    WINDOWS_RESERVED_NAMES.contains(&device.as_str())
}

/// `String.prototype.indexOf(ch, from)` over a char slice, `-1` when absent.
fn index_of(p: &[char], ch: char, from: usize) -> isize {
    p.iter()
        .skip(from)
        .position(|&c| c == ch)
        .map(|i| (i + from) as isize)
        .unwrap_or(-1)
}

/// Port of Node's `normalizeString`: resolve `.`/`..` and collapse repeated
/// separators, emitting `separator`-joined segments.
fn normalize_string(
    path: &[char],
    allow_above_root: bool,
    flavor: Flavor,
    separator: char,
) -> String {
    let mut res: Vec<char> = Vec::new();
    let mut last_segment_length: isize = 0;
    let mut last_slash: isize = -1;
    let mut dots: isize = 0;
    let mut code: char = '\0';
    let len = path.len() as isize;
    let mut i: isize = 0;
    while i <= len {
        if i < len {
            code = path[i as usize];
        } else if flavor.is_sep(code) {
            break;
        } else {
            code = '/';
        }

        if flavor.is_sep(code) {
            if last_slash == i - 1 || dots == 1 {
                // NOOP — an empty segment or a bare `.`.
            } else if dots == 2 {
                let rl = res.len() as isize;
                if rl < 2
                    || last_segment_length != 2
                    || res[res.len() - 1] != '.'
                    || res[res.len() - 2] != '.'
                {
                    if rl > 2 {
                        let last_slash_index = rl - last_segment_length - 1;
                        if last_slash_index == -1 {
                            res.clear();
                            last_segment_length = 0;
                        } else {
                            res.truncate(last_slash_index as usize);
                            let li = res
                                .iter()
                                .rposition(|&c| c == separator)
                                .map(|p| p as isize)
                                .unwrap_or(-1);
                            last_segment_length = res.len() as isize - 1 - li;
                        }
                        last_slash = i;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if rl != 0 {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(separator);
                    }
                    res.push('.');
                    res.push('.');
                    last_segment_length = 2;
                }
            } else {
                if !res.is_empty() {
                    res.push(separator);
                }
                res.extend_from_slice(&path[(last_slash + 1) as usize..i as usize]);
                last_segment_length = i - last_slash - 1;
            }
            last_slash = i;
            dots = 0;
        } else if code == '.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res.into_iter().collect()
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".into())
}

// ---------------------------------------------------------------------------
// resolve
// ---------------------------------------------------------------------------

/// `path.resolve(p)` against the current directory, for callers outside the JS
/// dispatcher. `process.argv[1]` is the resolved entry script, not the spelling
/// the user typed (`node ./x.js` reports `/cwd/x.js`), and reusing the ported
/// resolver keeps that agreeing with what `path.resolve` reports in-language.
pub(crate) fn resolve_one(p: &str) -> String {
    resolve_posix(&[p.to_string()])
}

fn resolve(flavor: Flavor, args: &[String]) -> String {
    match flavor {
        Flavor::Posix => resolve_posix(args),
        Flavor::Win32 => resolve_win32(args),
    }
}

fn resolve_posix(args: &[String]) -> String {
    if args.is_empty() || (args.len() == 1 && (args[0].is_empty() || args[0] == ".")) {
        let c = cwd();
        if c.starts_with('/') {
            return c;
        }
    }
    let mut resolved = String::new();
    let mut absolute = false;
    for p in args.iter().rev() {
        if absolute {
            break;
        }
        if p.is_empty() {
            continue;
        }
        resolved = std::format!("{p}/{resolved}");
        absolute = p.starts_with('/');
    }
    if !absolute {
        let c = cwd();
        resolved = std::format!("{c}/{resolved}");
        absolute = c.starts_with('/');
    }
    let out = normalize_string(&chars(&resolved), !absolute, Flavor::Posix, '/');
    if absolute {
        std::format!("/{out}")
    } else if out.is_empty() {
        ".".into()
    } else {
        out
    }
}

fn resolve_win32(args: &[String]) -> String {
    let f = Flavor::Win32;
    let mut resolved_device = String::new();
    let mut resolved_tail = String::new();
    let mut resolved_absolute = false;

    let mut i = args.len() as isize - 1;
    while i >= -1 {
        let path: Vec<char>;
        if i >= 0 {
            let p = &args[i as usize];
            if p.is_empty() {
                i -= 1;
                continue;
            }
            path = chars(p);
        } else if resolved_device.is_empty() {
            let c = cwd();
            // Fast path for the current directory. On a POSIX host Node
            // converts the cwd's forward slashes to backslashes here.
            if args.is_empty()
                || (args.len() == 1 && (args[0].is_empty() || args[0] == ".") && c.starts_with('/'))
            {
                return c.replace('/', "\\");
            }
            path = chars(&c);
        } else {
            // Windows keeps a per-drive cwd in a `=C:` env var; off Windows that
            // never exists, so Node falls back to `process.cwd()` and only
            // rewrites it to the bare drive root when the cwd names a DIFFERENT
            // drive (i.e. it has a `\` at index 2). A POSIX cwd has no such `\`,
            // so it is used verbatim — which is what `path.win32.resolve('C:')`
            // reports on this host.
            let c = cwd();
            let cc = chars(&c);
            let drive_mismatch = str_of(&cc[..cc.len().min(2)]).to_lowercase()
                != resolved_device.to_lowercase()
                && cc.get(2) == Some(&'\\');
            path = if drive_mismatch {
                chars(&std::format!("{resolved_device}\\"))
            } else {
                cc
            };
        }

        let len = path.len();
        let mut root_end: usize = 0;
        let mut device = String::new();
        let mut is_absolute = false;
        let code = *path.first().unwrap_or(&'\0');

        if len == 1 {
            if f.is_sep(code) {
                root_end = 1;
                is_absolute = true;
            }
        } else if f.is_sep(code) {
            is_absolute = true;
            if f.is_sep(path[1]) {
                let mut j = 2usize;
                let mut last = j;
                while j < len && !f.is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    let first_part = str_of(&path[last..j]);
                    last = j;
                    while j < len && f.is_sep(path[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !f.is_sep(path[j]) {
                            j += 1;
                        }
                        if j == len || j != last {
                            if first_part != "." && first_part != "?" {
                                device =
                                    std::format!("\\\\{first_part}\\{}", str_of(&path[last..j]));
                                root_end = j;
                            } else {
                                device = std::format!("\\\\{first_part}");
                                root_end = 4;
                            }
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if is_device_root(code) && path[1] == ':' {
            device = str_of(&path[..2]);
            root_end = 2;
            if len > 2 && f.is_sep(path[2]) {
                is_absolute = true;
                root_end = 3;
            }
        }

        if !device.is_empty() {
            if !resolved_device.is_empty() {
                if device.to_lowercase() != resolved_device.to_lowercase() {
                    i -= 1;
                    continue;
                }
            } else {
                resolved_device = device;
            }
        }

        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            resolved_tail = std::format!("{}\\{resolved_tail}", str_of(&path[root_end.min(len)..]));
            resolved_absolute = is_absolute;
            if is_absolute && !resolved_device.is_empty() {
                break;
            }
        }
        i -= 1;
    }

    let tail = normalize_string(&chars(&resolved_tail), !resolved_absolute, f, '\\');
    if resolved_absolute {
        std::format!("{resolved_device}{}{tail}", '\\')
    } else {
        let joined = std::format!("{resolved_device}{tail}");
        if joined.is_empty() {
            ".".into()
        } else {
            joined
        }
    }
}

// ---------------------------------------------------------------------------
// normalize
// ---------------------------------------------------------------------------

fn normalize(flavor: Flavor, path: &[char]) -> String {
    match flavor {
        Flavor::Posix => normalize_posix(path),
        Flavor::Win32 => normalize_win32(path),
    }
}

fn normalize_posix(path: &[char]) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let is_absolute = path[0] == '/';
    let trailing = path[path.len() - 1] == '/';
    let out = normalize_string(path, !is_absolute, Flavor::Posix, '/');
    if out.is_empty() {
        return if is_absolute {
            "/".into()
        } else if trailing {
            "./".into()
        } else {
            ".".into()
        };
    }
    let out = if trailing {
        std::format!("{out}/")
    } else {
        out
    };
    if is_absolute {
        std::format!("/{out}")
    } else {
        out
    }
}

fn normalize_win32(path: &[char]) -> String {
    let f = Flavor::Win32;
    let len = path.len();
    if len == 0 {
        return ".".into();
    }
    let mut root_end: usize = 0;
    let mut device: Option<String> = None;
    let mut is_absolute = false;
    let code = path[0];

    if len == 1 {
        return if code == '/' {
            "\\".into()
        } else {
            str_of(path)
        };
    }
    if f.is_sep(code) {
        is_absolute = true;
        if f.is_sep(path[1]) {
            let mut j = 2usize;
            let mut last = j;
            while j < len && !f.is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first_part = str_of(&path[last..j]);
                last = j;
                while j < len && f.is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !f.is_sep(path[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if first_part == "." || first_part == "?" {
                            device = Some(std::format!("\\\\{first_part}"));
                            root_end = 4;
                            let colon_index = index_of(path, ':', 0);
                            let end = (colon_index + 1).max(0) as usize;
                            let possible: Vec<char> = if end >= 4 && end <= len {
                                path[4..end].to_vec()
                            } else {
                                Vec::new()
                            };
                            if is_reserved_name(&possible, possible.len() as isize - 1) {
                                device = Some(std::format!("\\\\?\\{}", str_of(&possible)));
                                root_end = 4 + possible.len();
                            }
                        } else if j == len {
                            return std::format!("\\\\{first_part}\\{}\\", str_of(&path[last..]));
                        } else {
                            device =
                                Some(std::format!("\\\\{first_part}\\{}", str_of(&path[last..j])));
                            root_end = j;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else {
        let colon_index = index_of(path, ':', 0);
        if colon_index > 0 {
            if is_device_root(code) && colon_index == 1 {
                device = Some(str_of(&path[..2]));
                root_end = 2;
                if len > 2 && f.is_sep(path[2]) {
                    is_absolute = true;
                    root_end = 3;
                }
            } else if is_reserved_name(path, colon_index) {
                device = Some(str_of(&path[..(colon_index + 1) as usize]));
                root_end = (colon_index + 1) as usize;
            }
        }
    }

    let mut tail = if root_end < len {
        normalize_string(&path[root_end..], !is_absolute, f, '\\')
    } else {
        String::new()
    };
    if tail.is_empty() && !is_absolute {
        tail = ".".into();
    }
    if !tail.is_empty() && f.is_sep(path[len - 1]) {
        tail.push('\\');
    }
    if !is_absolute && device.is_none() && path.contains(&':') {
        // CVE-2024-36139: a relative path must never normalize into something
        // Windows would read as drive-absolute.
        let tc = chars(&tail);
        if tc.len() >= 2 && is_device_root(tc[0]) && tc[1] == ':' {
            return std::format!(".\\{tail}");
        }
        let mut index = index_of(path, ':', 0);
        while index != -1 {
            if index == len as isize - 1 || f.is_sep(path[(index + 1) as usize]) {
                return std::format!(".\\{tail}");
            }
            index = index_of(path, ':', (index + 1) as usize);
        }
    }
    if is_reserved_name(path, index_of(path, ':', 0)) {
        return std::format!(".\\{}{tail}", device.unwrap_or_default());
    }
    match device {
        None => {
            if is_absolute {
                std::format!("\\{tail}")
            } else {
                tail
            }
        }
        Some(d) => {
            if is_absolute {
                std::format!("{d}\\{tail}")
            } else {
                std::format!("{d}{tail}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// isAbsolute / join
// ---------------------------------------------------------------------------

fn is_absolute(flavor: Flavor, path: &[char]) -> bool {
    let len = path.len();
    if len == 0 {
        return false;
    }
    match flavor {
        Flavor::Posix => path[0] == '/',
        Flavor::Win32 => {
            flavor.is_sep(path[0])
                || (len > 2 && is_device_root(path[0]) && path[1] == ':' && flavor.is_sep(path[2]))
        }
    }
}

fn join(flavor: Flavor, args: &[String]) -> String {
    let parts: Vec<&String> = args.iter().filter(|a| !a.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    match flavor {
        Flavor::Posix => {
            let joined = parts
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("/");
            normalize_posix(&chars(&joined))
        }
        Flavor::Win32 => join_win32(&parts),
    }
}

fn join_win32(parts: &[&String]) -> String {
    let f = Flavor::Win32;
    let first_part = chars(parts[0]);
    let mut joined: String = parts
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\\");

    // Avoid turning a plain absolute path into a UNC path, but keep an
    // intentional UNC prefix (`//server`) intact.
    let mut needs_replace = true;
    let mut slash_count = 0usize;
    if f.is_sep(*first_part.first().unwrap_or(&'\0')) {
        slash_count += 1;
        let first_len = first_part.len();
        if first_len > 1 && f.is_sep(first_part[1]) {
            slash_count += 1;
            if first_len > 2 {
                if f.is_sep(first_part[2]) {
                    slash_count += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }
    if needs_replace {
        let jc = chars(&joined);
        while slash_count < jc.len() && f.is_sep(jc[slash_count]) {
            slash_count += 1;
        }
        if slash_count >= 2 {
            joined = std::format!("\\{}", str_of(&jc[slash_count..]));
        }
    }

    // A reserved device name anywhere in the path suppresses normalization.
    let jc = chars(&joined);
    let mut segments: Vec<String> = Vec::new();
    let mut part = String::new();
    let mut i = 0usize;
    while i < jc.len() {
        if jc[i] == '\\' {
            if !part.is_empty() {
                segments.push(std::mem::take(&mut part));
            }
            part.clear();
            while i + 1 < jc.len() && jc[i + 1] == '\\' {
                i += 1;
            }
        } else {
            part.push(jc[i]);
        }
        i += 1;
    }
    if !part.is_empty() {
        segments.push(part);
    }
    if segments.iter().any(|p| {
        let pc = chars(p);
        let ci = index_of(&pc, ':', 0);
        ci != -1 && is_reserved_name(&pc, ci)
    }) {
        return joined.replace('/', "\\");
    }

    normalize_win32(&chars(&joined))
}

// ---------------------------------------------------------------------------
// relative
// ---------------------------------------------------------------------------

fn relative(flavor: Flavor, from: &[char], to: &[char]) -> String {
    match flavor {
        Flavor::Posix => relative_posix(from, to),
        Flavor::Win32 => relative_win32(from, to),
    }
}

fn relative_posix(from_in: &[char], to_in: &[char]) -> String {
    if from_in == to_in {
        return String::new();
    }
    let from = chars(&resolve_posix(&[str_of(from_in)]));
    let to = chars(&resolve_posix(&[str_of(to_in)]));
    if from == to {
        return String::new();
    }

    let from_start = 1isize;
    let from_end = from.len() as isize;
    let from_len = from_end - from_start;
    let to_start = 1isize;
    let to_len = to.len() as isize - to_start;

    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0isize;
    while i < length {
        let fc = from[(from_start + i) as usize];
        if fc != to[(to_start + i) as usize] {
            break;
        } else if fc == '/' {
            last_common_sep = i;
        }
        i += 1;
    }
    if i == length {
        if to_len > length {
            if to[(to_start + i) as usize] == '/' {
                return str_of(&to[(to_start + i + 1) as usize..]);
            }
            if i == 0 {
                return str_of(&to[(to_start + i) as usize..]);
            }
        } else if from_len > length {
            if from[(from_start + i) as usize] == '/' {
                last_common_sep = i;
            } else if i == 0 {
                last_common_sep = 0;
            }
        }
    }

    let mut out = String::new();
    let mut k = from_start + last_common_sep + 1;
    while k <= from_end {
        if k == from_end || from[k as usize] == '/' {
            out.push_str(if out.is_empty() { ".." } else { "/.." });
        }
        k += 1;
    }
    std::format!(
        "{out}{}",
        str_of(&to[(to_start + last_common_sep) as usize..])
    )
}

fn relative_win32(from_in: &[char], to_in: &[char]) -> String {
    if from_in == to_in {
        return String::new();
    }
    let from_orig = resolve_win32(&[str_of(from_in)]);
    let to_orig = resolve_win32(&[str_of(to_in)]);
    if from_orig == to_orig {
        return String::new();
    }
    let from_lc = from_orig.to_lowercase();
    let to_lc = to_orig.to_lowercase();
    if from_lc == to_lc {
        return String::new();
    }

    let from_orig_c = chars(&from_orig);
    let to_orig_c = chars(&to_orig);
    let from = chars(&from_lc);
    let to = chars(&to_lc);

    // A case-fold that changed the length (e.g. `İ`) invalidates index-parallel
    // scanning, so Node falls back to segment-wise comparison.
    if from_orig_c.len() != from.len() || to_orig_c.len() != to.len() {
        let mut from_split: Vec<String> = from_orig.split('\\').map(str::to_string).collect();
        let mut to_split: Vec<String> = to_orig.split('\\').map(str::to_string).collect();
        if from_split.last().is_some_and(String::is_empty) {
            from_split.pop();
        }
        if to_split.last().is_some_and(String::is_empty) {
            to_split.pop();
        }
        let from_len = from_split.len();
        let to_len = to_split.len();
        let length = from_len.min(to_len);
        let mut i = 0usize;
        while i < length {
            if from_split[i].to_lowercase() != to_split[i].to_lowercase() {
                break;
            }
            i += 1;
        }
        if i == 0 {
            return to_orig;
        } else if i == length {
            if to_len > length {
                return to_split[i..].join("\\");
            }
            if from_len > length {
                return "..\\".repeat(from_len - 1 - i) + "..";
            }
            return String::new();
        }
        return "..\\".repeat(from_len - i) + &to_split[i..].join("\\");
    }

    let mut from_start = 0isize;
    while (from_start as usize) < from.len() && from[from_start as usize] == '\\' {
        from_start += 1;
    }
    let mut from_end = from.len() as isize;
    while from_end - 1 > from_start && from[(from_end - 1) as usize] == '\\' {
        from_end -= 1;
    }
    let from_len = from_end - from_start;

    let mut to_start = 0isize;
    while (to_start as usize) < to.len() && to[to_start as usize] == '\\' {
        to_start += 1;
    }
    let mut to_end = to.len() as isize;
    while to_end - 1 > to_start && to[(to_end - 1) as usize] == '\\' {
        to_end -= 1;
    }
    let to_len = to_end - to_start;

    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0isize;
    while i < length {
        let fc = from[(from_start + i) as usize];
        if fc != to[(to_start + i) as usize] {
            break;
        } else if fc == '\\' {
            last_common_sep = i;
        }
        i += 1;
    }

    if i != length {
        if last_common_sep == -1 {
            return to_orig;
        }
    } else {
        if to_len > length {
            if to[(to_start + i) as usize] == '\\' {
                return str_of(&to_orig_c[(to_start + i + 1) as usize..]);
            }
            if i == 2 {
                return str_of(&to_orig_c[(to_start + i) as usize..]);
            }
        }
        if from_len > length {
            if from[(from_start + i) as usize] == '\\' {
                last_common_sep = i;
            } else if i == 2 {
                last_common_sep = 3;
            }
        }
        if last_common_sep == -1 {
            last_common_sep = 0;
        }
    }

    let mut out = String::new();
    let mut k = from_start + last_common_sep + 1;
    while k <= from_end {
        if k == from_end || from[k as usize] == '\\' {
            out.push_str(if out.is_empty() { ".." } else { "\\.." });
        }
        k += 1;
    }

    to_start += last_common_sep;
    if !out.is_empty() {
        return std::format!(
            "{out}{}",
            str_of(&to_orig_c[to_start as usize..to_end as usize])
        );
    }
    if to_orig_c.get(to_start as usize) == Some(&'\\') {
        to_start += 1;
    }
    str_of(&to_orig_c[to_start as usize..to_end as usize])
}

// ---------------------------------------------------------------------------
// toNamespacedPath
// ---------------------------------------------------------------------------

fn to_namespaced_path(flavor: Flavor, path: &str) -> String {
    if flavor == Flavor::Posix || path.is_empty() {
        return path.to_string();
    }
    let resolved = resolve_win32(&[path.to_string()]);
    let rc = chars(&resolved);
    if rc.len() <= 2 {
        return path.to_string();
    }
    if rc[0] == '\\' {
        if rc[1] == '\\' && rc[2] != '?' && rc[2] != '.' {
            return std::format!("\\\\?\\UNC\\{}", str_of(&rc[2..]));
        }
    } else if is_device_root(rc[0]) && rc[1] == ':' && rc[2] == '\\' {
        return std::format!("\\\\?\\{resolved}");
    }
    resolved
}

// ---------------------------------------------------------------------------
// dirname / basename / extname
// ---------------------------------------------------------------------------

fn dirname(flavor: Flavor, path: &[char]) -> String {
    match flavor {
        Flavor::Posix => dirname_posix(path),
        Flavor::Win32 => dirname_win32(path),
    }
}

fn dirname_posix(path: &[char]) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let has_root = path[0] == '/';
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = path.len() as isize - 1;
    while i >= 1 {
        if path[i as usize] == '/' {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }
    if end == -1 {
        return if has_root { "/".into() } else { ".".into() };
    }
    if has_root && end == 1 {
        return "//".into();
    }
    str_of(&path[..end as usize])
}

fn dirname_win32(path: &[char]) -> String {
    let f = Flavor::Win32;
    let len = path.len();
    if len == 0 {
        return ".".into();
    }
    let mut root_end: isize = -1;
    let mut offset: usize = 0;
    let code = path[0];

    if len == 1 {
        return if f.is_sep(code) {
            str_of(path)
        } else {
            ".".into()
        };
    }

    if f.is_sep(code) {
        root_end = 1;
        offset = 1;
        if f.is_sep(path[1]) {
            let mut j = 2usize;
            let mut last = j;
            while j < len && !f.is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                last = j;
                while j < len && f.is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !f.is_sep(path[j]) {
                        j += 1;
                    }
                    if j == len {
                        return str_of(path);
                    }
                    if j != last {
                        root_end = j as isize + 1;
                        offset = j + 1;
                    }
                }
            }
        }
    } else if is_device_root(code) && path[1] == ':' {
        root_end = if len > 2 && f.is_sep(path[2]) { 3 } else { 2 };
        offset = root_end as usize;
    }

    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = len as isize - 1;
    while i >= offset as isize {
        if f.is_sep(path[i as usize]) {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }

    if end == -1 {
        if root_end == -1 {
            return ".".into();
        }
        end = root_end;
    }
    str_of(&path[..end as usize])
}

fn basename(flavor: Flavor, path: &[char], suffix: Option<&[char]>) -> String {
    let mut start: isize = 0;
    let mut end: isize = -1;
    let mut matched_slash = true;

    // A `C:` prefix is a root, not a trailing-separator candidate.
    if flavor == Flavor::Win32 && path.len() >= 2 && is_device_root(path[0]) && path[1] == ':' {
        start = 2;
    }

    if let Some(sfx) = suffix.filter(|s| !s.is_empty() && s.len() <= path.len()) {
        if sfx == path {
            return String::new();
        }
        let mut ext_idx: isize = sfx.len() as isize - 1;
        let mut first_non_slash_end: isize = -1;
        let mut i = path.len() as isize - 1;
        while i >= start {
            let code = path[i as usize];
            if flavor.is_sep(code) {
                if !matched_slash {
                    start = i + 1;
                    break;
                }
            } else {
                if first_non_slash_end == -1 {
                    matched_slash = false;
                    first_non_slash_end = i + 1;
                }
                if ext_idx >= 0 {
                    if code == sfx[ext_idx as usize] {
                        ext_idx -= 1;
                        if ext_idx == -1 {
                            end = i;
                        }
                    } else {
                        ext_idx = -1;
                        end = first_non_slash_end;
                    }
                }
            }
            i -= 1;
        }
        if start == end {
            end = first_non_slash_end;
        } else if end == -1 {
            end = path.len() as isize;
        }
        return str_of(&path[start.max(0) as usize..end.max(0) as usize]);
    }

    let mut i = path.len() as isize - 1;
    while i >= start {
        if flavor.is_sep(path[i as usize]) {
            if !matched_slash {
                start = i + 1;
                break;
            }
        } else if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        i -= 1;
    }
    if end == -1 {
        return String::new();
    }
    str_of(&path[start as usize..end as usize])
}

fn extname(flavor: Flavor, path: &[char]) -> String {
    let mut start: isize = 0;
    let mut start_dot: isize = -1;
    let mut start_part: isize = 0;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state: isize = 0;

    if flavor == Flavor::Win32 && path.len() >= 2 && path[1] == ':' && is_device_root(path[0]) {
        start = 2;
        start_part = 2;
    }

    let mut i = path.len() as isize - 1;
    while i >= start {
        let code = path[i as usize];
        if flavor.is_sep(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if code == '.' {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }

    if start_dot == -1
        || end == -1
        || pre_dot_state == 0
        || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1)
    {
        return String::new();
    }
    str_of(&path[start_dot as usize..end as usize])
}

// ---------------------------------------------------------------------------
// parse / format
// ---------------------------------------------------------------------------

fn new_parsed(root: &str, dir: &str, base: &str, ext: &str, name: &str) -> Value {
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

fn parse(flavor: Flavor, path: &[char]) -> Value {
    let (root, dir, base, ext, name) = match flavor {
        Flavor::Posix => parse_posix(path),
        Flavor::Win32 => parse_win32(path),
    };
    new_parsed(&root, &dir, &base, &ext, &name)
}

type Parsed = (String, String, String, String, String);

fn parse_posix(path: &[char]) -> Parsed {
    let mut root = String::new();
    let mut dir = String::new();
    let mut base = String::new();
    let mut ext = String::new();
    let mut name = String::new();
    if path.is_empty() {
        return (root, dir, base, ext, name);
    }
    let is_abs = path[0] == '/';
    let scan_start: isize = if is_abs {
        root = "/".into();
        1
    } else {
        0
    };

    let mut start_dot: isize = -1;
    let mut start_part: isize = 0;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state: isize = 0;
    let mut i = path.len() as isize - 1;
    while i >= scan_start {
        let code = path[i as usize];
        if code == '/' {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if code == '.' {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }

    if end != -1 {
        let s = if start_part == 0 && is_abs {
            1
        } else {
            start_part
        };
        if start_dot == -1
            || pre_dot_state == 0
            || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1)
        {
            base = str_of(&path[s as usize..end as usize]);
            name = base.clone();
        } else {
            name = str_of(&path[s as usize..start_dot as usize]);
            base = str_of(&path[s as usize..end as usize]);
            ext = str_of(&path[start_dot as usize..end as usize]);
        }
    }

    if start_part > 0 {
        dir = str_of(&path[..(start_part - 1) as usize]);
    } else if is_abs {
        dir = "/".into();
    }
    (root, dir, base, ext, name)
}

fn parse_win32(path: &[char]) -> Parsed {
    let f = Flavor::Win32;
    let mut root = String::new();
    let mut dir = String::new();
    let mut base = String::new();
    let mut ext = String::new();
    let mut name = String::new();
    let len = path.len();
    if len == 0 {
        return (root, dir, base, ext, name);
    }

    let mut root_end: usize = 0;
    let code = path[0];

    if len == 1 {
        if f.is_sep(code) {
            root = str_of(path);
            dir = root.clone();
            return (root, dir, base, ext, name);
        }
        base = str_of(path);
        name = base.clone();
        return (root, dir, base, ext, name);
    }

    if f.is_sep(code) {
        root_end = 1;
        if f.is_sep(path[1]) {
            let mut j = 2usize;
            let mut last = j;
            while j < len && !f.is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                last = j;
                while j < len && f.is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !f.is_sep(path[j]) {
                        j += 1;
                    }
                    if j == len {
                        root_end = j;
                    } else if j != last {
                        root_end = j + 1;
                    }
                }
            }
        }
    } else if is_device_root(code) && path[1] == ':' {
        if len <= 2 {
            root = str_of(path);
            dir = root.clone();
            return (root, dir, base, ext, name);
        }
        root_end = 2;
        if f.is_sep(path[2]) {
            if len == 3 {
                root = str_of(path);
                dir = root.clone();
                return (root, dir, base, ext, name);
            }
            root_end = 3;
        }
    }
    if root_end > 0 {
        root = str_of(&path[..root_end]);
    }

    let mut start_dot: isize = -1;
    let mut start_part: isize = root_end as isize;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state: isize = 0;
    let mut i = len as isize - 1;
    while i >= root_end as isize {
        let c = path[i as usize];
        if f.is_sep(c) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if c == '.' {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }

    if end != -1 {
        if start_dot == -1
            || pre_dot_state == 0
            || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1)
        {
            base = str_of(&path[start_part as usize..end as usize]);
            name = base.clone();
        } else {
            name = str_of(&path[start_part as usize..start_dot as usize]);
            base = str_of(&path[start_part as usize..end as usize]);
            ext = str_of(&path[start_dot as usize..end as usize]);
        }
    }

    if start_part > 0 && start_part != root_end as isize {
        dir = str_of(&path[..(start_part - 1) as usize]);
    } else {
        dir = root.clone();
    }
    (root, dir, base, ext, name)
}

/// Port of Node's `_format(sep, pathObject)`.
fn format(flavor: Flavor, obj: Option<&Value>) -> String {
    let Some(obj) = obj else { return String::new() };
    let get = |k: &str| {
        with_host(|h| match h.get(obj) {
            Some(crate::host::JsObj::Object(p)) => match p.get(k) {
                Some(Value::Undef) | None => String::new(),
                Some(v) => h.str_of(v),
            },
            _ => String::new(),
        })
    };
    let root = get("root");
    let dir_raw = get("dir");
    let base_raw = get("base");
    let base = if !base_raw.is_empty() {
        base_raw
    } else {
        let ext = get("ext");
        let ext = if ext.is_empty() {
            String::new()
        } else if ext.starts_with('.') {
            ext
        } else {
            std::format!(".{ext}")
        };
        std::format!("{}{ext}", get("name"))
    };
    let dir = if !dir_raw.is_empty() {
        dir_raw
    } else {
        root.clone()
    };
    if dir.is_empty() {
        return base;
    }
    if dir == root {
        std::format!("{dir}{base}")
    } else {
        std::format!("{dir}{}{base}", flavor.sep())
    }
}

/// `path.matchesGlob(path, pattern)` — whether `path` matches the glob `pattern`.
/// Supports `*` (within a segment), `**` (across `/`), `?`, `[...]` classes, and
/// top-level `{a,b}` brace alternatives — the minimatch-style subset Node uses.
/// Under the win32 flavor both slashes are separators, matching Node's
/// `matchGlobPattern(path, pattern, /* windows */ true)`.
fn matches_glob(flavor: Flavor, path: &str, pattern: &str) -> bool {
    let (path, pattern) = match flavor {
        Flavor::Posix => (path.to_string(), pattern.to_string()),
        Flavor::Win32 => (path.replace('\\', "/"), pattern.replace('\\', "/")),
    };
    let text: Vec<char> = path.chars().collect();
    expand_braces(&pattern)
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
        let (Some(close), false) = (close, commas.is_empty()) else {
            continue;
        };
        let prefix: String = chars[..i].iter().collect();
        let suffix: String = chars[close + 1..].iter().collect();
        let mut bounds = vec![i];
        bounds.extend(&commas);
        bounds.push(close);
        let mut out = Vec::new();
        for w in bounds.windows(2) {
            let alt: String = chars[w[0] + 1..w[1]].iter().collect();
            out.extend(expand_braces(&std::format!("{prefix}{alt}{suffix}")));
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
