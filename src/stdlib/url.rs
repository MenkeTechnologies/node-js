//! Node `url` module: the WHATWG `URL` class (global + `require('url').URL`) and
//! the legacy `url.parse`. A `URL` instance stores its components as data
//! properties (so `u.hostname` reads directly) plus a `@@native = "URL"` tag for
//! `toString`.

use super::arg_str;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const MODULE_METHODS: &[&str] = &[
    "parse",
    "format",
    "fileURLToPath",
    "fileURLToPathBuffer",
    "pathToFileURL",
    "domainToASCII",
    "domainToUnicode",
    "urlToHttpOptions",
    "resolve",
    "resolveObject",
];

/// Parsed URL components.
struct Parts {
    protocol: String,
    username: String,
    password: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
}

impl Parts {
    fn host(&self) -> String {
        if self.port.is_empty() {
            self.hostname.clone()
        } else {
            format!("{}:{}", self.hostname, self.port)
        }
    }
    fn origin(&self) -> String {
        if self.hostname.is_empty() {
            "null".into()
        } else {
            format!("{}//{}", self.protocol, self.host())
        }
    }
    fn href(&self) -> String {
        let auth = if self.username.is_empty() {
            String::new()
        } else if self.password.is_empty() {
            format!("{}@", self.username)
        } else {
            format!("{}:{}@", self.username, self.password)
        };
        format!(
            "{}//{auth}{}{}{}{}",
            self.protocol,
            self.host(),
            self.pathname,
            self.search,
            self.hash
        )
    }
}

/// Parse an absolute URL. Returns `None` if there is no `scheme://`.
fn parse_absolute(input: &str) -> Option<Parts> {
    let (scheme, rest) = input.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    // authority is up to the first '/', '?' or '#'.
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let mut tail = &rest[auth_end..];

    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (u, h),
        None => ("", authority),
    };
    let (username, password) = match userinfo.split_once(':') {
        Some((u, p)) => (u.to_string(), p.to_string()),
        None => (userinfo.to_string(), String::new()),
    };
    let (hostname, port) = match hostport.split_once(':') {
        Some((h, p)) => (h.to_string(), p.to_string()),
        None => (hostport.to_string(), String::new()),
    };

    let hash = match tail.find('#') {
        Some(i) => {
            let h = tail[i..].to_string();
            tail = &tail[..i];
            h
        }
        None => String::new(),
    };
    let search = match tail.find('?') {
        Some(i) => {
            let s = tail[i..].to_string();
            tail = &tail[..i];
            s
        }
        None => String::new(),
    };
    let pathname = if tail.is_empty() { "/".to_string() } else { tail.to_string() };

    Some(Parts {
        protocol: format!("{scheme}:"),
        username,
        password,
        hostname,
        port,
        pathname,
        search,
        hash,
    })
}

/// `new URL(input[, base])`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    let input = arg_str(args, 0);
    let parts = parse_absolute(&input)
        .or_else(|| {
            // A base makes a relative input absolute (path replacement only).
            if args.len() > 1 {
                let base = arg_str(args, 1);
                parse_absolute(&base).map(|mut b| {
                    if input.starts_with('/') {
                        b.pathname = input.clone();
                    } else {
                        b.pathname = format!("/{input}");
                    }
                    b.search.clear();
                    b.hash.clear();
                    b
                })
            } else {
                None
            }
        })
        .ok_or_else(|| format!("TypeError: Invalid URL: {input}"))?;
    Ok(build(&parts))
}

fn build(p: &Parts) -> Value {
    // Build the `URLSearchParams` snapshot BEFORE the allocating `with_host` below
    // (never nest `with_host`); it is stored as the `searchParams` data property so
    // `url.searchParams.get(...)` reads it directly. It is a static snapshot of the
    // query at construction — mutating it does not rewrite `url.href`.
    let query = p.search.strip_prefix('?').unwrap_or(&p.search);
    let search_params = make_search_params(&parse_query(query));
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("URL"));
        m.insert("href".into(), h.new_str(p.href()));
        m.insert("origin".into(), h.new_str(p.origin()));
        m.insert("protocol".into(), h.new_str(p.protocol.clone()));
        m.insert("username".into(), h.new_str(p.username.clone()));
        m.insert("password".into(), h.new_str(p.password.clone()));
        m.insert("host".into(), h.new_str(p.host()));
        m.insert("hostname".into(), h.new_str(p.hostname.clone()));
        m.insert("port".into(), h.new_str(p.port.clone()));
        m.insert("pathname".into(), h.new_str(p.pathname.clone()));
        m.insert("search".into(), h.new_str(p.search.clone()));
        m.insert("searchParams".into(), search_params);
        m.insert("hash".into(), h.new_str(p.hash.clone()));
        h.new_object(m)
    })
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "parse" => Ok(legacy_parse(&arg_str(args, 0))),
        "format" => Ok(with_host(|h| {
            // format(URL|object): if it's a native URL, return its href.
            let v = args.first().cloned().unwrap_or(Value::Undef);
            let s = match h.get(&v) {
                Some(JsObj::Object(p)) => p.get("href").map(|x| h.str_of(x)).unwrap_or_default(),
                _ => h.str_of(&v),
            };
            h.new_str(s)
        })),
        // `url.fileURLToPath(url)` — a `file:` URL/string → a filesystem path
        // (percent-decoded). POSIX best-effort: any authority (host) is accepted
        // but not re-prefixed; Windows drive/UNC rewriting is not modeled.
        "fileURLToPath" => file_url_to_path(args).map(|s| with_host(|h| h.new_str(s))),
        // Same, but returns the path as a `Buffer`.
        "fileURLToPathBuffer" => file_url_to_path(args).map(|s| super::buffer::from_bytes(s.as_bytes())),
        // `url.pathToFileURL(path)` → a `URL` instance with a `file:` href.
        "pathToFileURL" => Ok(path_to_file_url(&arg_str(args, 0))),
        // `url.domainToASCII` / `url.domainToUnicode` — delegate to the punycode
        // codec; an ASCII-only domain passes through unchanged, an invalid domain
        // yields "" (matching Node, which never throws here).
        "domainToASCII" => Ok(punycode_domain(args, true)),
        "domainToUnicode" => Ok(punycode_domain(args, false)),
        // `url.urlToHttpOptions(URL)` → an options object for http/https.request.
        "urlToHttpOptions" => Ok(url_to_http_options(&args.first().cloned().unwrap_or(Value::Undef))),
        // Legacy `url.resolve(from, to)` — RFC 3986 §5 reference resolution.
        "resolve" => {
            let from = arg_str(args, 0);
            let to = arg_str(args, 1);
            Ok(with_host(|h| h.new_str(legacy_resolve(&from, &to))))
        }
        // Legacy `url.resolveObject(from, to)` — the resolved URL as a parsed object.
        "resolveObject" => {
            let from = arg_str(args, 0);
            let to = arg_str(args, 1);
            Ok(legacy_parse(&legacy_resolve(&from, &to)))
        }
        _ => return None,
    })
}

/// Legacy `url.parse` — a plain object (not a `URL` instance), matching Node's
/// field set and insertion order.
fn legacy_parse(input: &str) -> Value {
    let p = parse_absolute(input);
    with_host(|h| {
        let mut m = IndexMap::new();
        let null = h.null();
        let opt = |h: &mut crate::host::JsHost, s: &str| if s.is_empty() { h.null() } else { h.new_str(s) };
        match p {
            Some(p) => {
                let auth = if p.username.is_empty() {
                    String::new()
                } else if p.password.is_empty() {
                    p.username.clone()
                } else {
                    format!("{}:{}", p.username, p.password)
                };
                m.insert("protocol".into(), h.new_str(p.protocol.clone()));
                m.insert("slashes".into(), Value::Bool(true));
                m.insert("auth".into(), opt(h, &auth));
                m.insert("host".into(), h.new_str(p.host()));
                m.insert("port".into(), opt(h, &p.port));
                m.insert("hostname".into(), h.new_str(p.hostname.clone()));
                m.insert("hash".into(), opt(h, &p.hash));
                m.insert("search".into(), opt(h, &p.search));
                m.insert("query".into(), opt(h, p.search.trim_start_matches('?')));
                m.insert("pathname".into(), h.new_str(p.pathname.clone()));
                m.insert("path".into(), h.new_str(format!("{}{}", p.pathname, p.search)));
                m.insert("href".into(), h.new_str(p.href()));
            }
            None => {
                m.insert("protocol".into(), null.clone());
                m.insert("slashes".into(), null.clone());
                m.insert("auth".into(), null.clone());
                m.insert("host".into(), null.clone());
                m.insert("port".into(), null.clone());
                m.insert("hostname".into(), null.clone());
                m.insert("hash".into(), null.clone());
                m.insert("search".into(), null.clone());
                m.insert("query".into(), null.clone());
                m.insert("pathname".into(), h.new_str(input));
                m.insert("path".into(), h.new_str(input));
                m.insert("href".into(), h.new_str(input));
            }
        }
        h.new_object(m)
    })
}

/// `URL` instance methods (component reads are plain data properties).
pub fn instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    match method {
        "toString" | "toJSON" => Ok(with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("href").cloned().unwrap_or(Value::Undef),
            _ => Value::Undef,
        })),
        _ => Err(crate::host::type_error(&format!("url.{method} is not a function"))),
    }
}

// ── file:/legacy URL helpers ─────────────────────────────────────────────────

/// The `href` string of a value: for a native `URL` its stored `href`, else the
/// value coerced to a string (so both `URL` objects and strings are accepted).
fn url_href(v: &Value) -> String {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get("@@native").map(|x| h.str_of(x)).as_deref() {
            Some("URL") => p.get("href").map(|x| h.str_of(x)).unwrap_or_default(),
            _ => h.str_of(v),
        },
        _ => h.str_of(v),
    })
}

/// `fileURLToPath` core: `file://[host]/path` → decoded `/path`.
fn file_url_to_path(args: &[Value]) -> Result<String, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    let href = url_href(&v);
    let rest = href
        .strip_prefix("file://")
        .ok_or_else(|| crate::host::type_error("The URL must be of scheme file"))?;
    // The authority runs up to the first '/'; the remainder is the path.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(i) => &rest[i..],
        None => "/",
    };
    Ok(percent_decode(path))
}

/// `pathToFileURL(path)` → a `URL` instance whose href is `file://` + the
/// percent-encoded (path-set) path.
fn path_to_file_url(path: &str) -> Value {
    let enc = encode_path_component(path);
    let pathname = if enc.starts_with('/') { enc } else { format!("/{enc}") };
    let parts = Parts {
        protocol: "file:".into(),
        username: String::new(),
        password: String::new(),
        hostname: String::new(),
        port: String::new(),
        pathname,
        search: String::new(),
        hash: String::new(),
    };
    build(&parts)
}

/// `domainToASCII` (`ascii = true`) / `domainToUnicode` — via the punycode codec.
fn punycode_domain(args: &[Value], ascii: bool) -> Value {
    let method = if ascii { "toASCII" } else { "toUnicode" };
    match super::punycode::call(method, args) {
        Some(Ok(v)) => v,
        _ => with_host(|h| h.new_str("")),
    }
}

/// `urlToHttpOptions(URL)` → `{ protocol, hostname, hash, search, pathname, path,
/// href[, port][, auth] }`, mirroring Node's field set and IPv6 bracket-stripping.
fn url_to_http_options(v: &Value) -> Value {
    let get = |key: &str| -> String {
        with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get(key).map(|x| h.str_of(x)).unwrap_or_default(),
            _ => String::new(),
        })
    };
    let protocol = get("protocol");
    let mut hostname = get("hostname");
    if hostname.starts_with('[') && hostname.ends_with(']') && hostname.len() >= 2 {
        hostname = hostname[1..hostname.len() - 1].to_string();
    }
    let hash = get("hash");
    let search = get("search");
    let pathname = get("pathname");
    let href = get("href");
    let port = get("port");
    let username = get("username");
    let password = get("password");
    let path = format!("{pathname}{search}");
    let auth = if username.is_empty() && password.is_empty() {
        None
    } else {
        Some(format!("{}:{}", percent_decode(&username), percent_decode(&password)))
    };
    let port_num = if port.is_empty() { None } else { port.parse::<f64>().ok() };
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("protocol".into(), h.new_str(protocol));
        m.insert("hostname".into(), h.new_str(hostname));
        m.insert("hash".into(), h.new_str(hash));
        m.insert("search".into(), h.new_str(search));
        m.insert("pathname".into(), h.new_str(pathname));
        m.insert("path".into(), h.new_str(path));
        m.insert("href".into(), h.new_str(href));
        if let Some(n) = port_num {
            m.insert("port".into(), Value::Float(n));
        }
        if let Some(a) = auth {
            m.insert("auth".into(), h.new_str(a));
        }
        h.new_object(m)
    })
}

/// Percent-decode a URL component (`%XX` → byte, then UTF-8 lossy). Unlike the
/// form decoder this leaves `+` literal (a file path may legitimately contain it).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode a path for a `file:` URL: keep the unreserved + sub-delim set
/// and `/ : @`, encode everything else (space, `# ? %` `< > "` etc.).
fn encode_path_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'/' | b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'('
                    | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@'
            );
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_upper(b >> 4));
            out.push(hex_upper(b & 0x0f));
        }
    }
    out
}

// ── legacy url.resolve — RFC 3986 §5 reference resolution ─────────────────────

/// A URI split into its five RFC-3986 components.
struct UriRef {
    scheme: Option<String>,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

/// Split a URI reference into its components (RFC 3986 Appendix B), by hand.
fn split_uri(input: &str) -> UriRef {
    let mut rest = input;
    // scheme: leading ALPHA *(ALPHA/DIGIT/+/-/.) then ':' — but only if that ':'
    // precedes the first '/', '?' or '#'.
    let mut scheme = None;
    if let Some(colon) = rest.find(':') {
        let cand = &rest[..colon];
        let scheme_ok = !cand.is_empty()
            && cand.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && cand.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && cand.find(['/', '?', '#']).is_none();
        if scheme_ok {
            scheme = Some(cand.to_string());
            rest = &rest[colon + 1..];
        }
    }
    let mut fragment = None;
    if let Some(h) = rest.find('#') {
        fragment = Some(rest[h + 1..].to_string());
        rest = &rest[..h];
    }
    let mut query = None;
    if let Some(q) = rest.find('?') {
        query = Some(rest[q + 1..].to_string());
        rest = &rest[..q];
    }
    let mut authority = None;
    if let Some(r) = rest.strip_prefix("//") {
        let end = r.find('/').unwrap_or(r.len());
        authority = Some(r[..end].to_string());
        rest = &r[end..];
    }
    UriRef { scheme, authority, path: rest.to_string(), query, fragment }
}

/// Merge a relative path onto a base (RFC 3986 §5.2.3).
fn merge_paths(base: &UriRef, ref_path: &str) -> String {
    if base.authority.is_some() && base.path.is_empty() {
        format!("/{ref_path}")
    } else {
        match base.path.rfind('/') {
            Some(i) => format!("{}{ref_path}", &base.path[..=i]),
            None => ref_path.to_string(),
        }
    }
}

/// Drop the last path segment of `output` (used by `..` handling).
fn remove_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(pos) => output.truncate(pos),
        None => output.clear(),
    }
}

/// Remove `.`/`..` dot-segments from a path (RFC 3986 §5.2.4).
fn remove_dot_segments(path: &str) -> String {
    let mut input = path.to_string();
    let mut output = String::new();
    while !input.is_empty() {
        if let Some(r) = input.strip_prefix("../") {
            input = r.to_string();
        } else if let Some(r) = input.strip_prefix("./") {
            input = r.to_string();
        } else if let Some(r) = input.strip_prefix("/./") {
            input = format!("/{r}");
        } else if input == "/." {
            input = "/".to_string();
        } else if let Some(r) = input.strip_prefix("/../") {
            input = format!("/{r}");
            remove_last_segment(&mut output);
        } else if input == "/.." {
            input = "/".to_string();
            remove_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input.clear();
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..].find('/').map(|i| start + i).unwrap_or(input.len());
            output.push_str(&input[..end]);
            input.drain(..end);
        }
    }
    output
}

/// RFC 3986 §5.2.2 transform-references: resolve `r` against `base`.
fn resolve_ref(base: &UriRef, r: &UriRef) -> UriRef {
    if r.scheme.is_some() {
        return UriRef {
            scheme: r.scheme.clone(),
            authority: r.authority.clone(),
            path: remove_dot_segments(&r.path),
            query: r.query.clone(),
            fragment: r.fragment.clone(),
        };
    }
    let (authority, path, query) = if r.authority.is_some() {
        (r.authority.clone(), remove_dot_segments(&r.path), r.query.clone())
    } else if r.path.is_empty() {
        let q = if r.query.is_some() { r.query.clone() } else { base.query.clone() };
        (base.authority.clone(), base.path.clone(), q)
    } else if r.path.starts_with('/') {
        (base.authority.clone(), remove_dot_segments(&r.path), r.query.clone())
    } else {
        (base.authority.clone(), remove_dot_segments(&merge_paths(base, &r.path)), r.query.clone())
    };
    UriRef { scheme: base.scheme.clone(), authority, path, query, fragment: r.fragment.clone() }
}

/// Recompose a URI from its components (RFC 3986 §5.3).
fn recompose(u: &UriRef) -> String {
    let mut s = String::new();
    if let Some(sc) = &u.scheme {
        s.push_str(sc);
        s.push(':');
    }
    if let Some(a) = &u.authority {
        s.push_str("//");
        s.push_str(a);
    }
    s.push_str(&u.path);
    if let Some(q) = &u.query {
        s.push('?');
        s.push_str(q);
    }
    if let Some(f) = &u.fragment {
        s.push('#');
        s.push_str(f);
    }
    s
}

/// Legacy `url.resolve(from, to)` — RFC 3986 reference resolution end-to-end.
fn legacy_resolve(from: &str, to: &str) -> String {
    recompose(&resolve_ref(&split_uri(from), &split_uri(to)))
}

// ── URLSearchParams ──────────────────────────────────────────────────────────
//
// A `URLSearchParams` is a plain object tagged `@@native = "URLSearchParams"`
// whose ordered `[key, value]` pairs live in a hidden `@@pairs` array (each entry
// a 2-element `[key, value]` array of strings). All string coercion happens up
// front; methods mutate a plain `Vec<(String, String)>` and write it back.

/// Method names dispatched through `search_params_call` (for `instance_has_method`
/// wiring in `stdlib::mod`; `@@iterator` makes `[...params]` / `for..of` work).
pub const SEARCH_PARAMS_METHODS: &[&str] = &[
    "get", "getAll", "has", "set", "append", "delete", "keys", "values", "entries",
    "forEach", "toString", "sort", "@@iterator",
];

/// Build a `URLSearchParams` native object from ordered key/value pairs.
fn make_search_params(pairs: &[(String, String)]) -> Value {
    with_host(|h| {
        let items: Vec<Value> = pairs
            .iter()
            .map(|(k, v)| {
                let kv = vec![h.new_str(k.clone()), h.new_str(v.clone())];
                h.new_array(kv)
            })
            .collect();
        let arr = h.new_array(items);
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("URLSearchParams"));
        m.insert("@@pairs".into(), arr);
        h.new_object(m)
    })
}

/// Read the ordered `(key, value)` pairs out of a `URLSearchParams`.
fn pairs_of(recv: &Value) -> Vec<(String, String)> {
    with_host(|h| {
        let items: Vec<Value> = match h.get(recv) {
            Some(JsObj::Object(p)) => match p.get("@@pairs").and_then(|a| h.get(a)) {
                Some(JsObj::Array(items)) => items.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        items
            .iter()
            .map(|it| match h.get(it) {
                Some(JsObj::Array(kv)) => {
                    let kv = kv.clone();
                    let k = kv.first().map(|x| h.str_of(x)).unwrap_or_default();
                    let v = kv.get(1).map(|x| h.str_of(x)).unwrap_or_default();
                    (k, v)
                }
                _ => (h.str_of(it), String::new()),
            })
            .collect()
    })
}

/// Overwrite a `URLSearchParams`' backing `@@pairs` array.
fn set_pairs(recv: &Value, pairs: &[(String, String)]) {
    with_host(|h| {
        let items: Vec<Value> = pairs
            .iter()
            .map(|(k, v)| {
                let kv = vec![h.new_str(k.clone()), h.new_str(v.clone())];
                h.new_array(kv)
            })
            .collect();
        let arr = h.new_array(items);
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@pairs".into(), arr);
        }
    });
}

/// `new URLSearchParams([init])` — from a query string, an object, an iterable of
/// `[key, value]` pairs, another `URLSearchParams`, or empty.
pub fn construct_search_params(args: &[Value]) -> Result<Value, String> {
    let pairs = match args.first() {
        None => Vec::new(),
        Some(v) if matches!(v, Value::Undef) || with_host(|h| h.is_null(v)) => Vec::new(),
        Some(v) => pairs_from_init(v),
    };
    Ok(make_search_params(&pairs))
}

fn pairs_from_init(v: &Value) -> Vec<(String, String)> {
    // Copy of another URLSearchParams.
    if super::native_tag(v).as_deref() == Some("URLSearchParams") {
        return pairs_of(v);
    }
    // Query string (a leading `?` is stripped, matching the URL/WHATWG parser).
    if let Some(s) = with_host(|h| h.as_str(v)) {
        return parse_query(s.strip_prefix('?').unwrap_or(&s));
    }
    with_host(|h| match h.get(v) {
        // Iterable of `[key, value]` pairs.
        Some(JsObj::Array(items)) => {
            let items = items.clone();
            items
                .iter()
                .map(|it| match h.get(it) {
                    Some(JsObj::Array(kv)) => {
                        let kv = kv.clone();
                        let k = kv.first().map(|x| h.str_of(x)).unwrap_or_default();
                        let val = kv.get(1).map(|x| h.str_of(x)).unwrap_or_default();
                        (k, val)
                    }
                    _ => (h.str_of(it), String::new()),
                })
                .collect()
        }
        // Plain object: own enumerable entries (hidden `@@` keys excluded).
        Some(JsObj::Object(p)) => {
            let entries: Vec<(String, Value)> = p
                .iter()
                .filter(|(k, _)| !k.starts_with("@@"))
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect();
            entries.into_iter().map(|(k, val)| (k, h.str_of(&val))).collect()
        }
        _ => Vec::new(),
    })
}

/// `URLSearchParams` instance methods.
pub fn search_params_call(recv: &Value, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "get" => {
            let name = arg_str(args, 0);
            match pairs_of(recv).into_iter().find(|(k, _)| *k == name) {
                Some((_, v)) => Ok(with_host(|h| h.new_str(v))),
                None => Ok(with_host(|h| h.null())),
            }
        }
        "getAll" => {
            let name = arg_str(args, 0);
            let vals: Vec<String> = pairs_of(recv)
                .into_iter()
                .filter(|(k, _)| *k == name)
                .map(|(_, v)| v)
                .collect();
            Ok(with_host(|h| {
                let items = vals.into_iter().map(|v| h.new_str(v)).collect();
                h.new_array(items)
            }))
        }
        "has" => {
            let name = arg_str(args, 0);
            let pairs = pairs_of(recv);
            let found = if args.len() > 1 {
                let val = arg_str(args, 1);
                pairs.iter().any(|(k, v)| *k == name && *v == val)
            } else {
                pairs.iter().any(|(k, _)| *k == name)
            };
            Ok(Value::Bool(found))
        }
        "append" => {
            let mut pairs = pairs_of(recv);
            pairs.push((arg_str(args, 0), arg_str(args, 1)));
            set_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "set" => {
            let name = arg_str(args, 0);
            let val = arg_str(args, 1);
            let mut pairs = pairs_of(recv);
            // Set the first pair named `name` to `val`, remove any others; append
            // if none existed (WHATWG `set`).
            let mut seen = false;
            pairs.retain_mut(|(k, v)| {
                if *k == name {
                    if seen {
                        false
                    } else {
                        *v = val.clone();
                        seen = true;
                        true
                    }
                } else {
                    true
                }
            });
            if !seen {
                pairs.push((name, val));
            }
            set_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "delete" => {
            let name = arg_str(args, 0);
            let mut pairs = pairs_of(recv);
            if args.len() > 1 {
                let val = arg_str(args, 1);
                pairs.retain(|(k, v)| !(*k == name && *v == val));
            } else {
                pairs.retain(|(k, _)| *k != name);
            }
            set_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "sort" => {
            let mut pairs = pairs_of(recv);
            // Stable sort by key, comparing UTF-16 code units (WHATWG `sort`).
            pairs.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            set_pairs(recv, &pairs);
            Ok(Value::Undef)
        }
        "toString" => {
            let s = pairs_of(recv)
                .iter()
                .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            Ok(with_host(|h| h.new_str(s)))
        }
        "keys" => {
            let pairs = pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(k, _)| h.new_str(k)).collect();
                h.alloc(JsObj::Iter { items, idx: 0 })
            }))
        }
        "values" => {
            let pairs = pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(_, v)| h.new_str(v)).collect();
                h.alloc(JsObj::Iter { items, idx: 0 })
            }))
        }
        "entries" | "@@iterator" => {
            let pairs = pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        let kv = vec![h.new_str(k), h.new_str(v)];
                        h.new_array(kv)
                    })
                    .collect();
                h.alloc(JsObj::Iter { items, idx: 0 })
            }))
        }
        "forEach" => {
            let cb = args.first().cloned().unwrap_or(Value::Undef);
            let this_arg = args.get(1).cloned();
            // Materialize pairs (releasing the host borrow) before re-entrant invoke.
            for (k, v) in pairs_of(recv) {
                let (value, name) = with_host(|h| (h.new_str(v), h.new_str(k)));
                crate::host::invoke(&cb, vec![value, name, recv.clone()], this_arg.clone())?;
            }
            Ok(Value::Undef)
        }
        _ => Err(crate::host::type_error(&format!(
            "urlSearchParams.{method} is not a function"
        ))),
    }
}

/// Parse an `application/x-www-form-urlencoded` string into ordered pairs.
fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .map(|seg| match seg.split_once('=') {
            Some((k, v)) => (form_decode(k), form_decode(v)),
            None => (form_decode(seg), String::new()),
        })
        .collect()
}

/// Decode one `application/x-www-form-urlencoded` component (`+` → space,
/// `%XX` → byte, then UTF-8 lossy).
fn form_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi << 4) | lo);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encode one `application/x-www-form-urlencoded` component: space → `+`, the
/// unreserved set `A-Za-z0-9 * - . _` verbatim, every other byte percent-encoded.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            _ if b.is_ascii_alphanumeric() => out.push(b as char),
            _ => {
                out.push('%');
                out.push(hex_upper(b >> 4));
                out.push(hex_upper(b & 0x0f));
            }
        }
    }
    out
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_upper(n: u8) -> char {
    char::from_digit(n as u32, 16).unwrap().to_ascii_uppercase()
}
