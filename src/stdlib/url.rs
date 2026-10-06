//! Node `url` module: the WHATWG `URL` class (global + `require('url').URL`) and
//! the legacy `url.parse`. Parsing and the component setters are the URL
//! Standard's, through the `url` crate (its basic URL parser and `quirks`
//! setters). A `URL` instance keeps each component in a hidden `@@<name>` slot
//! read by the accessors on `URL.prototype`; a setter rewrites every slot from
//! the re-serialized URL ([`set_component`]), and the `searchParams` it carries
//! holds an `@@ownerUrl` back-reference so its own mutations rewrite the query
//! in the other direction.

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
/// The component names a `URL` exposes as writable ACCESSORS on its prototype.
///
/// Assigning one has to rewrite the DERIVED fields — `href`, `host` and
/// `origin` — which are stored alongside rather than computed on read. Without
/// that, `u.pathname = '/p'` read back as `/p` while `u.href` still showed the
/// old path, so the object disagreed with itself.
///
/// `host` and `href` are here too, and both need more than a write: `host`
/// carries the port, and assigning `href` REPLACES the whole URL. Neither was
/// settable, so `u.href = 'http://x/y'` stored a string that every other
/// property then contradicted.
pub const COMPONENTS: &[&str] = &[
    "protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash",
    "href",
];

/// Whether `name` is a `URL` component whose assignment must refresh the
/// derived fields.
pub fn is_component(name: &str) -> bool {
    COMPONENTS.contains(&name)
}

/// The components a URL's parse produced, as the getters report them.
fn parts_of(u: &url::Url) -> Parts {
    let delimited = |lead: char, s: Option<&str>| match s {
        Some(s) if !s.is_empty() => format!("{lead}{s}"),
        _ => String::new(),
    };
    Parts {
        protocol: format!("{}:", u.scheme()),
        username: u.username().to_string(),
        password: u.password().unwrap_or("").to_string(),
        hostname: u.host_str().unwrap_or("").to_string(),
        port: u.port().map(|p| p.to_string()).unwrap_or_default(),
        pathname: u.path().to_string(),
        search: delimited('?', u.query()),
        hash: delimited('#', u.fragment()),
        authority: u.has_authority(),
        href: Some(u.as_str().to_string()),
    }
}

/// Assign one `URL` component through the URL Standard's setter for it — the
/// state-override parse `url::quirks` implements — and rewrite every stored
/// field from the result. A value the setter rejects leaves the URL as it was
/// (`u.port = 'abc'`), as in node; only `href` throws, since it reparses the
/// whole URL.
pub fn set_component(url_obj: &Value, key: &str, v: &Value) -> Result<(), String> {
    let value = crate::host::to_string_value(v).map(|s| with_host(|h| h.str_of(&s)))?;
    let href = read_slot(url_obj, "@@href");
    let Ok(mut u) = url::Url::parse(&href) else {
        return Ok(());
    };
    use url::quirks;
    match key {
        "href" => {
            u = url::Url::parse(&value).map_err(|_| {
                crate::host::plain_coded_error_with(
                    "TypeError",
                    "ERR_INVALID_URL",
                    "Invalid URL",
                    &[("input", value.as_str())],
                )
            })?;
        }
        "protocol" => {
            let _ = quirks::set_protocol(&mut u, &value);
        }
        "username" => {
            let _ = quirks::set_username(&mut u, &value);
        }
        "password" => {
            let _ = quirks::set_password(&mut u, &value);
        }
        "host" => {
            let _ = quirks::set_host(&mut u, &value);
        }
        "hostname" => {
            let _ = quirks::set_hostname(&mut u, &value);
        }
        "port" => {
            let _ = quirks::set_port(&mut u, &value);
        }
        "pathname" => quirks::set_pathname(&mut u, &value),
        "search" => quirks::set_search(&mut u, &value),
        "hash" => quirks::set_hash(&mut u, &value),
        _ => return Ok(()),
    }
    store_parts(url_obj, &parts_of(&u), true);
    Ok(())
}

fn read_slot(obj: &Value, key: &str) -> String {
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p.get(key).map(|v| h.str_of(v)).unwrap_or_default(),
        _ => String::new(),
    })
}

/// Write every component of `p` into the URL's hidden slots. `sync_params`
/// rewrites the attached `URLSearchParams` IN PLACE (one object per URL for its
/// life); it is false when that object is the one pushing its own edit back.
fn store_parts(url_obj: &Value, p: &Parts, sync_params: bool) {
    if sync_params {
        let query = p.search.strip_prefix('?').unwrap_or(&p.search).to_string();
        let params = with_host(|h| match h.get(url_obj) {
            Some(JsObj::Object(o)) => o.get("@@searchParams").cloned(),
            _ => None,
        });
        if let Some(params) = params {
            write_pairs(&params, &parse_query(&query));
        }
    }
    with_host(|h| {
        let vals = [
            ("@@href", h.new_str(p.href())),
            ("@@origin", h.new_str(p.origin())),
            ("@@protocol", h.new_str(p.protocol.clone())),
            ("@@username", h.new_str(p.username.clone())),
            ("@@password", h.new_str(p.password.clone())),
            ("@@host", h.new_str(p.host())),
            ("@@hostname", h.new_str(p.hostname.clone())),
            ("@@port", h.new_str(p.port.clone())),
            ("@@pathname", h.new_str(p.pathname.clone())),
            ("@@search", h.new_str(p.search.clone())),
            ("@@hash", h.new_str(p.hash.clone())),
            ("@@authority", h.new_str(if p.authority { "true" } else { "false" })),
        ];
        if let Some(JsObj::Object(o)) = h.get_mut(url_obj) {
            for (k, v) in vals {
                o.insert(k.to_string(), v);
            }
        }
    });
}

struct Parts {
    protocol: String,
    username: String,
    password: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    /// Whether the URL has an authority (`//`): an opaque-path URL such as
    /// `mailto:a@b` has none and serializes without the slashes.
    authority: bool,
    /// The parser's own serialization, when the parts came from it; setters
    /// re-serialize from the components instead.
    href: Option<String>,
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
        // A `blob:` URL's origin is that of the URL in its path, when that one
        // is http(s) (URL Standard, "origin" for scheme `blob`).
        if self.protocol == "blob:" {
            return match url::Url::parse(&self.pathname) {
                Ok(inner) if matches!(inner.scheme(), "http" | "https") => {
                    inner.origin().ascii_serialization()
                }
                _ => "null".into(),
            };
        }
        // Only a special scheme with a network host has a tuple origin; every
        // other URL (`foo://h/`, `redis://h:1/`, `file:///x`) is opaque: `null`.
        let scheme = self.protocol.strip_suffix(':').unwrap_or(&self.protocol);
        if self.hostname.is_empty() || special_port(scheme).is_none() {
            "null".into()
        } else {
            format!("{}//{}", self.protocol, self.host())
        }
    }
    fn href(&self) -> String {
        if let Some(h) = &self.href {
            return h.clone();
        }
        if !self.authority && self.hostname.is_empty() {
            // The URL Standard's serializer: no host, so no `//`; a path that
            // would then read as one gets the `/.` prefix.
            let dot = if self.pathname.starts_with("//") { "/." } else { "" };
            return format!("{}{dot}{}{}{}", self.protocol, self.pathname, self.search, self.hash);
        }
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

/// Whether `scheme` is one of the WHATWG "special" schemes, whose parsing
/// normalizes backslashes and drops a default port.
fn special_port(scheme: &str) -> Option<&'static str> {
    match scheme {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        "ftp" => Some("21"),
        _ => None,
    }
}

/// The WHATWG URL parser (`url` crate, an implementation of the URL Standard's
/// basic URL parser) over `input`, resolved against `base` when one is given.
///
/// Replaces a hand-rolled split on `://` that knew nothing of opaque paths
/// (`mailto:a@b`, `data:…`, `x:` were all `Invalid URL`), scheme-relative input
/// (`//h/p` was appended to the base's path), a base's query surviving a
/// fragment-only reference, or a same-scheme `http:foo` reference.
fn parse_whatwg(input: &str, base: Option<&str>) -> Option<Parts> {
    let base = match base {
        Some(b) => Some(url::Url::parse(b).ok()?),
        None => None,
    };
    let u = url::Url::options().base_url(base.as_ref()).parse(input).ok()?;
    Some(parts_of(&u))
}

/// `new URL(input[, base])`.
pub fn construct(args: &[Value]) -> Result<Value, String> {
    // Both arguments go through ToString, so an object's own `toString` is
    // what gets parsed (`new URL('x', { toString() { return 'http://a/' } })`).
    let to_str = |v: &Value| {
        crate::host::to_string_value(v).map(|s| crate::host::with_host(|h| h.str_of(&s)))
    };
    let input = match args.first() {
        Some(v) => to_str(v)?,
        None => "undefined".to_string(),
    };
    // An explicit `undefined` base is no base at all.
    let base = match args.get(1) {
        Some(Value::Undef) | None => None,
        Some(v) => Some(to_str(v)?),
    };
    let parts = parse_whatwg(&input, base.as_deref())
        // Node's message is the bare `Invalid URL` and it carries
        // `code === 'ERR_INVALID_URL'`; the input is exposed as `err.input`, not
        // appended to the text. `url_legacy::invalid_url` was already emitting
        // the current form — this site was the one still hardcoding an older one.
        // node also hangs the input (and the base, when one was passed) off the
        // error as `err.input` / `err.base`.
        .ok_or_else(|| {
            let mut fields = vec![("input", input.as_str())];
            if let Some(b) = &base {
                fields.push(("base", b.as_str()));
            }
            crate::host::plain_coded_error_with(
                "TypeError",
                "ERR_INVALID_URL",
                "Invalid URL",
                &fields,
            )
        })?;
    Ok(build(&parts))
}

/// Percent-encode `s` for one URL component, per the WHATWG percent-encode sets.
///
/// None of this was happening: `new URL('https://a.b/a b?c=d e').href` came back
/// with the spaces intact, which is not a valid URL and does not round-trip.
///
/// The sets below were derived by feeding every ASCII character through node
/// v26.8.1 in each position rather than transcribed, since the spec's sets and
/// what a parser actually emits differ around the component delimiters. Every
/// C0 control, `%7F`, and every non-ASCII byte is encoded in all four; a byte
/// already part of a valid `%XX` escape is left alone so re-parsing a URL does
/// not double-encode it.
fn percent_encode(s: &str, extra: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // An existing escape passes through untouched.
        if b == b'%' && i + 2 < bytes.len() + 1 {
            let hex = bytes.get(i + 1..i + 3);
            if hex.is_some_and(|h| h.iter().all(|c| c.is_ascii_hexdigit())) {
                out.push('%');
                out.push(bytes[i + 1] as char);
                out.push(bytes[i + 2] as char);
                i += 3;
                continue;
            }
        }
        if b < 0x20 || b == 0x7f || b >= 0x80 || extra.as_bytes().contains(&b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

/// The four component encode sets, as measured against node.
const PATH_SET: &str = " \"<>^`{}";
const QUERY_SET: &str = " \"'<>";
const FRAGMENT_SET: &str = " \"<>`";
const USERINFO_SET: &str = " \";<=>@[]^`{|}";

fn build(p: &Parts) -> Value {
    // Percent-encode each component once, here, so `href()` and every
    // individual property report the same normalized text. The host arrives
    // already canonical from the host parser in `parse_absolute`; lower-casing
    // it again here also folded a non-special scheme's opaque host, which
    // node keeps as written (`foo://Host/`).
    let p = &Parts {
        protocol: p.protocol.clone(),
        username: percent_encode(&p.username, USERINFO_SET),
        password: percent_encode(&p.password, USERINFO_SET),
        hostname: p.hostname.clone(),
        port: p.port.clone(),
        pathname: percent_encode(&p.pathname, PATH_SET),
        search: percent_encode(&p.search, QUERY_SET),
        hash: percent_encode(&p.hash, FRAGMENT_SET),
        authority: p.authority,
        href: p.href.clone(),
    };
    // Build the `URLSearchParams` BEFORE the allocating `with_host` below (never
    // nest `with_host`); it is stored as the `searchParams` data property so
    // `url.searchParams.get(...)` reads it directly. It is LIVE, not a snapshot:
    // it gets an `@@ownerUrl` back-reference below so that mutating it rewrites
    // this URL's `search` and `href`.
    let query = p.search.strip_prefix('?').unwrap_or(&p.search);
    let search_params = make_search_params(&parse_query(query));
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("@@native".into(), h.new_str("URL"));
        m.insert("@@href".into(), h.new_str(p.href()));
        m.insert("@@origin".into(), h.new_str(p.origin()));
        m.insert("@@protocol".into(), h.new_str(p.protocol.clone()));
        m.insert("@@username".into(), h.new_str(p.username.clone()));
        m.insert("@@password".into(), h.new_str(p.password.clone()));
        m.insert("@@host".into(), h.new_str(p.host()));
        m.insert("@@hostname".into(), h.new_str(p.hostname.clone()));
        m.insert("@@port".into(), h.new_str(p.port.clone()));
        m.insert("@@pathname".into(), h.new_str(p.pathname.clone()));
        m.insert("@@search".into(), h.new_str(p.search.clone()));
        m.insert("@@searchParams".into(), search_params.clone());
        m.insert("@@hash".into(), h.new_str(p.hash.clone()));
        m.insert("@@authority".into(), h.new_str(if p.authority { "true" } else { "false" }));
        let obj = h.new_object(m);
        // Hidden, and set after the URL exists so the two can point at each other.
        if let Some(JsObj::Object(sp)) = h.get_mut(&search_params) {
            sp.insert("@@ownerUrl".into(), obj.clone());
        }
        obj
    })
}

/// Statics on the `URL` CLASS — distinct from [`MODULE_METHODS`], which are the
/// legacy `require('url')` functions.
///
/// `createObjectURL`/`revokeObjectURL` are absent because `Blob` is not
/// implemented; they would have nothing to register.
pub const STATIC_METHODS: &[&str] = &["canParse", "parse"];

/// `URL.canParse(input[, base])` / `URL.parse(input[, base])`.
///
/// Both are the non-throwing form of the constructor: `canParse` reports
/// whether parsing succeeds, `parse` returns the `URL` or `null`. Neither
/// existed, so `URL.canParse` was a TypeError rather than a boolean.
pub fn static_call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let parsed = construct(args);
    Some(match method {
        "canParse" => Ok(Value::Bool(parsed.is_ok())),
        "parse" => Ok(parsed.unwrap_or_else(|_| with_host(|h| h.null()))),
        _ => return None,
    })
}

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match method {
        "parse" => legacy_parse(args).map(|u| super::url_legacy::to_js(&u)),
        "format" => super::url_legacy::format_value(
            &args.first().cloned().unwrap_or(Value::Undef),
            &args.get(1).cloned().unwrap_or(Value::Undef),
        ),
        // `url.fileURLToPath(url)` — a `file:` URL/string → a filesystem path
        // (percent-decoded). POSIX best-effort: any authority (host) is accepted
        // but not re-prefixed; Windows drive/UNC rewriting is not modeled.
        "fileURLToPath" => file_url_to_path(args).map(|s| with_host(|h| h.new_str(s))),
        // Same, but returns the path as a `Buffer`.
        "fileURLToPathBuffer" => {
            file_url_to_path(args).map(|s| super::buffer::from_bytes(s.as_bytes()))
        }
        // `url.pathToFileURL(path)` → a `URL` instance with a `file:` href.
        "pathToFileURL" => Ok(path_to_file_url(&arg_str(args, 0))),
        // `url.domainToASCII` / `url.domainToUnicode` — delegate to the punycode
        // codec; an ASCII-only domain passes through unchanged, an invalid domain
        // yields "" (matching Node, which never throws here).
        "domainToASCII" => Ok(punycode_domain(args, true)),
        "domainToUnicode" => Ok(punycode_domain(args, false)),
        // `url.urlToHttpOptions(URL)` → an options object for http/https.request.
        "urlToHttpOptions" => Ok(url_to_http_options(
            &args.first().cloned().unwrap_or(Value::Undef),
        )),
        // Legacy `url.resolve(from, to)` — `urlParse(from, false, true)
        // .resolve(to)`: both sides parsed with `slashesDenoteHost`, resolved by
        // the `Url.prototype.resolveObject` port, then formatted.
        "resolve" => legacy_resolve_object(args)
            .map(|u| with_host(|h| h.new_str(u.href.unwrap_or_default()))),
        // Legacy `url.resolveObject(from, to)` — the same resolution, returned
        // as the parsed object. An empty `from` hands `to` back untouched.
        "resolveObject" => {
            if !args.first().is_some_and(|v| with_host(|h| h.truthy(v))) {
                return Some(Ok(args.get(1).cloned().unwrap_or(Value::Undef)));
            }
            legacy_resolve_object(args).map(|u| super::url_legacy::to_js(&u))
        }
        _ => return None,
    })
}

/// Legacy `url.parse(urlString[, parseQueryString[, slashesDenoteHost]])`.
/// Emits the one-shot `DEP0169` deprecation warning, exactly as Node's
/// `urlParse` does, then delegates to the `Url.prototype.parse` port.
fn legacy_parse(args: &[Value]) -> Result<super::url_legacy::Url, String> {
    emit_url_parse_deprecation();
    let input = arg_str(args, 0);
    let truthy = |i: usize| {
        args.get(i)
            .map(|v| with_host(|h| h.truthy(v)))
            .unwrap_or(false)
    };
    super::url_legacy::parse(&input, truthy(1), truthy(2))
}

/// `urlParse`'s one-time `DEP0169`, shared by `parse`, `resolve` and
/// `resolveObject` — all three go through `urlParse` in node.
fn emit_url_parse_deprecation() {
    super::process::emit_deprecation_warning(
        "DEP0169",
        "`url.parse()` behavior is not standardized and prone to errors that \
         have security implications. Use the WHATWG URL API instead. CVEs are \
         not issued for `url.parse()` vulnerabilities.",
    );
}

/// `urlParse(args[0], false, true).resolveObject(args[1])`, emitting the
/// one-shot `DEP0169` that `urlParse` raises.
fn legacy_resolve_object(args: &[Value]) -> Result<super::url_legacy::Url, String> {
    emit_url_parse_deprecation();
    let source = super::url_legacy::parse(&arg_str(args, 0), false, true)?;
    let relative = super::url_legacy::parse(&arg_str(args, 1), false, true)?;
    Ok(super::url_legacy::resolve_object(&source, relative))
}

/// `URL` instance methods (component reads are plain data properties).
pub fn instance_call(recv: &Value, method: &str, _args: &[Value]) -> Result<Value, String> {
    match method {
        "toString" | "toJSON" => Ok(with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => p.get("@@href").cloned().unwrap_or(Value::Undef),
            _ => Value::Undef,
        })),
        _ => Err(crate::host::type_error(&format!(
            "url.{method} is not a function"
        ))),
    }
}

// ── file:/legacy URL helpers ─────────────────────────────────────────────────

/// The `href` string of a value: for a native `URL` its stored `href`, else the
/// value coerced to a string (so both `URL` objects and strings are accepted).
fn url_href(v: &Value) -> String {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => match p.get("@@native").map(|x| h.str_of(x)).as_deref() {
            Some("URL") => p.get("@@href").map(|x| h.str_of(x)).unwrap_or_default(),
            _ => h.str_of(v),
        },
        _ => h.str_of(v),
    })
}

/// `fileURLToPath` core: `file://[host]/path` → decoded `/path`.
fn file_url_to_path(args: &[Value]) -> Result<String, String> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    let href = url_href(&v);
    let rest = href.strip_prefix("file://").ok_or_else(|| {
        crate::host::plain_coded_error(
            "TypeError",
            "ERR_INVALID_URL_SCHEME",
            "The URL must be of scheme file",
        )
    })?;
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
    let pathname = if enc.starts_with('/') {
        enc
    } else {
        format!("/{enc}")
    };
    let parts = Parts {
        protocol: "file:".into(),
        username: String::new(),
        password: String::new(),
        hostname: String::new(),
        port: String::new(),
        pathname,
        search: String::new(),
        hash: String::new(),
        authority: true,
        href: None,
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
    let protocol = get("@@protocol");
    let mut hostname = get("@@hostname");
    if hostname.starts_with('[') && hostname.ends_with(']') && hostname.len() >= 2 {
        hostname = hostname[1..hostname.len() - 1].to_string();
    }
    let hash = get("@@hash");
    let search = get("@@search");
    let pathname = get("@@pathname");
    let href = get("@@href");
    let port = get("@@port");
    let username = get("@@username");
    let password = get("@@password");
    let path = format!("{pathname}{search}");
    let auth = if username.is_empty() && password.is_empty() {
        None
    } else {
        Some(format!(
            "{}:{}",
            percent_decode(&username),
            percent_decode(&password)
        ))
    };
    let port_num = if port.is_empty() {
        None
    } else {
        port.parse::<f64>().ok()
    };
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
pub(crate) fn percent_decode(s: &str) -> String {
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
                b'/' | b'-'
                    | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
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

// ── URLSearchParams ──────────────────────────────────────────────────────────
//
// A `URLSearchParams` is a plain object tagged `@@native = "URLSearchParams"`
// whose ordered `[key, value]` pairs live in a hidden `@@pairs` array (each entry
// a 2-element `[key, value]` array of strings). All string coercion happens up
// front; methods mutate a plain `Vec<(String, String)>` and write it back.

/// Method names dispatched through `search_params_call` (for `instance_has_method`
/// wiring in `stdlib::mod`; `@@iterator` makes `[...params]` / `for..of` work).
pub const SEARCH_PARAMS_METHODS: &[&str] = &[
    "get",
    "getAll",
    "has",
    "set",
    "append",
    "delete",
    "keys",
    "values",
    "entries",
    "forEach",
    "toString",
    "sort",
    "@@iterator",
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
        // `size` is a prototype getter in the spec; kept in sync as a hidden own
        // property here, so it reads back without appearing in `Object.keys` or
        // `console.log`. `set_pairs` maintains it.
        m.insert("size".into(), Value::Float(pairs.len() as f64));
        let obj = h.new_object(m);
        h.hide_prop(&obj, "size");
        obj
    })
}

/// Serialize ordered pairs back into an `application/x-www-form-urlencoded`
/// query string — the inverse of [`parse_query`].
fn encode_query(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
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

/// Overwrite a `URLSearchParams`' backing `@@pairs` array, and push the new
/// query back to the `URL` that owns it if there is one.
///
/// A `URLSearchParams` reached through `url.searchParams` is LIVE in both
/// directions: `u.searchParams.set('b', '2')` has to rewrite `u.search` and
/// `u.href`. It was previously a detached snapshot, so the edit went nowhere.
fn set_pairs(recv: &Value, pairs: &[(String, String)]) {
    write_pairs(recv, pairs);
    let owner = with_host(|h| match h.get(recv) {
        Some(JsObj::Object(p)) => p.get("@@ownerUrl").cloned(),
        _ => None,
    });
    // URL Standard "update": the owner's query becomes the serialization,
    // or null when there are no pairs.
    if let Some(owner) = owner {
        let query = encode_query(pairs);
        if let Ok(mut u) = url::Url::parse(&read_slot(&owner, "@@href")) {
            u.set_query(if query.is_empty() { None } else { Some(&query) });
            store_parts(&owner, &parts_of(&u), false);
        }
    }
}

/// Write `pairs` into a `URLSearchParams` without notifying an owning `URL`.
fn write_pairs(recv: &Value, pairs: &[(String, String)]) {
    with_host(|h| {
        let items: Vec<Value> = pairs
            .iter()
            .map(|(k, v)| {
                let kv = vec![h.new_str(k.clone()), h.new_str(v.clone())];
                h.new_array(kv)
            })
            .collect();
        let arr = h.new_array(items);
        let n = Value::Float(pairs.len() as f64);
        if let Some(JsObj::Object(p)) = h.get_mut(recv) {
            p.insert("@@pairs".into(), arr);
            p.insert("size".into(), n);
        }
        h.hide_prop(recv, "size");
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
            entries
                .into_iter()
                .map(|(k, val)| (k, h.str_of(&val)))
                .collect()
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
            let s = encode_query(&pairs_of(recv));
            Ok(with_host(|h| h.new_str(s)))
        }
        "keys" => {
            let pairs = pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(k, _)| h.new_str(k)).collect();
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                })
            }))
        }
        "values" => {
            let pairs = pairs_of(recv);
            Ok(with_host(|h| {
                let items = pairs.into_iter().map(|(_, v)| h.new_str(v)).collect();
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                })
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
                h.alloc(JsObj::Iter {
                    items,
                    idx: 0,
                    array: None,
                })
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
