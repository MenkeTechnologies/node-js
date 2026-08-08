//! The legacy `url.parse` / `url.format` API — a faithful port of Node's
//! `lib/url.js` `Url.prototype.parse`, `Url.prototype.format` and `urlFormat`.
//!
//! This API predates (and disagrees with) the WHATWG `URL` parser in `url.rs`:
//! it is a hand-rolled scanner with its own whitespace trimming, backslash
//! rewriting, "simple path" fast path, auto-escaping table and slashed/hostless
//! protocol sets. Reimplementing it from the documentation produces a parser
//! that agrees on `http://host/path` and disagrees on everything else, so this
//! is a line-for-line port of the JS instead, sharing the same field names and
//! insertion order (`protocol, slashes, auth, host, port, hostname, hash,
//! search, query, pathname, path, href`).
//!
//! Node scans by UTF-16 code unit; this port scans by `char`. Every branch keys
//! off ASCII delimiters, so the emitted strings match.

use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Protocols whose `rest` is NOT auto-escaped (`javascript:`).
fn is_unsafe_protocol(lower_proto: &str) -> bool {
    matches!(lower_proto, "javascript" | "javascript:")
}

/// Protocols that never take a host, even after `//` (`javascript:`).
fn is_hostless_protocol(lower_proto: &str) -> bool {
    matches!(lower_proto, "javascript" | "javascript:")
}

/// Protocols that imply `//` in `format` and a `/` pathname in `parse`.
fn is_slashed_protocol(p: &str) -> bool {
    matches!(
        p,
        "http"
            | "http:"
            | "https"
            | "https:"
            | "ftp"
            | "ftp:"
            | "gopher"
            | "gopher:"
            | "file"
            | "file:"
            | "ws"
            | "ws:"
            | "wss"
            | "wss:"
    )
}

/// Node's `escapedCodes` table: the RFC 2396 delimiters + unwise characters
/// (plus `'`) that `autoEscapeStr` percent-encodes.
fn escaped_code(c: char) -> Option<&'static str> {
    Some(match c {
        '\t' => "%09",
        '\n' => "%0A",
        '\r' => "%0D",
        ' ' => "%20",
        '"' => "%22",
        '\'' => "%27",
        '<' => "%3C",
        '>' => "%3E",
        '\\' => "%5C",
        '^' => "%5E",
        '`' => "%60",
        '{' => "%7B",
        '|' => "%7C",
        '}' => "%7D",
        _ => return None,
    })
}

fn auto_escape_str(rest: &str) -> String {
    let mut out = String::with_capacity(rest.len());
    let mut escaped_any = false;
    for c in rest.chars() {
        match escaped_code(c) {
            Some(e) => {
                out.push_str(e);
                escaped_any = true;
            }
            None => out.push(c),
        }
    }
    if escaped_any {
        out
    } else {
        rest.to_string()
    }
}

/// JS `\s` for the purposes of `simplePathPattern`.
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// The whitespace class Node's trimming loop uses (`code < 33` plus NBSP/BOM).
fn is_trim_ws(c: char) -> bool {
    (c as u32) < 33 || c == '\u{a0}' || c == '\u{feff}'
}

/// The parsed legacy URL. Every field is `Option`, mirroring the `null`-initialized
/// `Url` instance; `slashes` is a tri-state (`None` = `null`).
#[derive(Default)]
pub struct Url {
    pub protocol: Option<String>,
    pub slashes: Option<bool>,
    pub auth: Option<String>,
    pub host: Option<String>,
    pub port: Option<String>,
    pub hostname: Option<String>,
    pub hash: Option<String>,
    pub search: Option<String>,
    /// `Some(Ok(raw))` when `parseQueryString` is off, `Some(Err(qs))` when it is
    /// on (the caller turns `qs` into an object via `querystring.parse`).
    pub query: Option<Result<String, String>>,
    pub pathname: Option<String>,
    pub path: Option<String>,
    pub href: Option<String>,
}

/// Port of `Url.prototype.parseHost`: split a trailing `:port` off `host`.
fn parse_host(u: &mut Url) {
    let Some(host) = u.host.clone() else { return };
    let hc: Vec<char> = host.chars().collect();
    // portPattern = /:[0-9]*$/
    let mut i = hc.len();
    while i > 0 && hc[i - 1].is_ascii_digit() {
        i -= 1;
    }
    let mut host = host;
    if i > 0 && hc[i - 1] == ':' {
        let port: String = hc[i..].iter().collect();
        if !port.is_empty() {
            u.port = Some(port);
        }
        host = hc[..i - 1].iter().collect();
    }
    if !host.is_empty() {
        u.hostname = Some(host);
    }
}

/// `/^[a-z0-9.+-]+:/i` — the matched protocol including the colon.
fn match_protocol(rest: &[char]) -> Option<String> {
    let mut i = 0;
    while i < rest.len() && (rest[i].is_ascii_alphanumeric() || matches!(rest[i], '.' | '+' | '-')) {
        i += 1;
    }
    if i > 0 && rest.get(i) == Some(&':') {
        Some(rest[..=i].iter().collect())
    } else {
        None
    }
}

/// `/^\/\/[^@/]+@[^@/]+/` — does `rest` look like `//user@host…`?
fn matches_host_pattern(rest: &[char]) -> bool {
    if rest.len() < 2 || rest[0] != '/' || rest[1] != '/' {
        return false;
    }
    let mut i = 2;
    let start = i;
    while i < rest.len() && rest[i] != '@' && rest[i] != '/' {
        i += 1;
    }
    if i == start || rest.get(i) != Some(&'@') {
        return false;
    }
    i += 1;
    let start = i;
    while i < rest.len() && rest[i] != '@' && rest[i] != '/' {
        i += 1;
    }
    i > start
}

/// `/^(\/\/?(?!\/)[^?\s]*)(\?[^\s]*)?$/` — returns `(group1, group2)` on a match.
fn match_simple_path(rest: &[char]) -> Option<(String, Option<String>)> {
    if rest.first() != Some(&'/') {
        return None;
    }
    // Greedy `\/?` then `(?!\/)`; backtracking to one slash also fails when the
    // next character is a slash, so `///…` never matches.
    let mut i = 1;
    if rest.get(1) == Some(&'/') {
        i = 2;
    }
    if rest.get(i) == Some(&'/') {
        return None;
    }
    let mut j = i;
    while j < rest.len() && rest[j] != '?' && !is_js_space(rest[j]) {
        j += 1;
    }
    let group1: String = rest[..j].iter().collect();
    if j == rest.len() {
        return Some((group1, None));
    }
    if rest[j] != '?' {
        // Trailing whitespace can't be consumed before the `$` anchor.
        return None;
    }
    if rest[j + 1..].iter().any(|&c| is_js_space(c)) {
        return None;
    }
    let group2: String = rest[j..].iter().collect();
    Some((group1, Some(group2)))
}

/// Node's `getHostname`: stop the hostname at the first character that can't
/// appear in one, moving the remainder into the path. `Err` mirrors the
/// `ERR_INVALID_ARG_VALUE` thrown for a leftover leading `:` (an invalid port).
fn get_hostname(u: &mut Url, rest: &str, hostname: &str, url: &str) -> Result<String, String> {
    for (i, c) in hostname.chars().enumerate() {
        if matches!(c, '/' | '\\' | '#' | '?' | ':') {
            if c == ':' {
                // Node's call site passes ERR_INVALID_ARG_VALUE's arguments in
                // an unusual order, so the rendered message interleaves the URL
                // and the reason exactly like this.
                return Err(std::format!(
                    "TypeError [ERR_INVALID_ARG_VALUE]: The argument 'url' {url}. \
                     Received 'Invalid port in url'"
                ));
            }
            let head: String = hostname.chars().take(i).collect();
            let tail: String = hostname.chars().skip(i).collect();
            u.hostname = Some(head);
            return Ok(std::format!("/{tail}{rest}"));
        }
    }
    Ok(rest.to_string())
}

fn is_ipv6_hostname(h: &str) -> bool {
    h.starts_with('[') && h.ends_with(']') && h.len() >= 2
}

fn has_forbidden_host_char(h: &str, ipv6: bool) -> bool {
    h.chars().any(|c| {
        matches!(
            c,
            '\0' | '\t' | '\n' | '\r' | ' ' | '#' | '%' | '/' | '<' | '>' | '?' | '@' | '\\' | '^'
                | '|'
        ) || (!ipv6 && matches!(c, ':' | '[' | ']'))
    })
}

/// Port of `Url.prototype.parse`. `slashes_denote_host` is the third `url.parse`
/// argument. Errors correspond to the JS `throw` sites.
pub fn parse(url: &str, parse_query_string: bool, slashes_denote_host: bool) -> Result<Url, String> {
    let mut u = Url::default();
    let uc: Vec<char> = url.chars().collect();

    // Trim outer whitespace and rewrite backslashes before the first `?`/`#`,
    // matching Chrome/IE/Opera (https://crbug.com/25916).
    let mut has_hash = false;
    let mut has_at = false;
    let mut start: isize = -1;
    let mut end: isize = -1;
    let mut rest = String::new();
    let mut last_pos: usize = 0;
    let mut in_ws = false;
    let mut split = false;
    for (i, &code) in uc.iter().enumerate() {
        let is_ws = is_trim_ws(code);
        if start == -1 {
            if is_ws {
                continue;
            }
            last_pos = i;
            start = i as isize;
        } else if in_ws {
            if !is_ws {
                end = -1;
                in_ws = false;
            }
        } else if is_ws {
            end = i as isize;
            in_ws = true;
        }

        if !split {
            match code {
                '@' => has_at = true,
                '#' => {
                    has_hash = true;
                    split = true;
                }
                '?' => split = true,
                '\\' => {
                    if i > last_pos {
                        rest.extend(&uc[last_pos..i]);
                    }
                    rest.push('/');
                    last_pos = i + 1;
                }
                _ => {}
            }
        } else if !has_hash && code == '#' {
            has_hash = true;
        }
    }

    if start != -1 {
        let s = start as usize;
        if last_pos == s {
            rest = if end == -1 {
                uc[s..].iter().collect()
            } else {
                uc[s..end as usize].iter().collect()
            };
        } else if end == -1 && last_pos < uc.len() {
            rest.extend(&uc[last_pos..]);
        } else if end != -1 && (last_pos as isize) < end {
            rest.extend(&uc[last_pos..end as usize]);
        }
    }

    let set_query = |u: &mut Url, raw: String| {
        u.query = Some(if parse_query_string {
            Err(raw)
        } else {
            Ok(raw)
        });
    };

    if !slashes_denote_host && !has_hash && !has_at {
        let rc: Vec<char> = rest.chars().collect();
        if let Some((g1, g2)) = match_simple_path(&rc) {
            u.path = Some(rest.clone());
            u.href = Some(rest.clone());
            u.pathname = Some(g1);
            match g2 {
                Some(q) => {
                    let raw: String = q.chars().skip(1).collect();
                    u.search = Some(q);
                    set_query(&mut u, raw);
                }
                None if parse_query_string => {
                    u.search = None;
                    u.query = Some(Err(String::new()));
                }
                None => {}
            }
            return Ok(u);
        }
    }

    let mut rc: Vec<char> = rest.chars().collect();
    let proto = match_protocol(&rc);
    let mut lower_proto = String::new();
    if let Some(p) = &proto {
        lower_proto = p.to_lowercase();
        u.protocol = Some(lower_proto.clone());
        rc = rc[p.chars().count()..].to_vec();
    }

    // `user@server` is always a host, and `//foo/bar` resolves as host=foo the
    // way a browser resolves a protocol-relative reference.
    let mut slashes = false;
    if slashes_denote_host || proto.is_some() || matches_host_pattern(&rc) {
        slashes = rc.first() == Some(&'/') && rc.get(1) == Some(&'/');
        if slashes && !(proto.is_some() && is_hostless_protocol(&lower_proto)) {
            rc = rc[2..].to_vec();
            u.slashes = Some(true);
        }
    }

    if !is_hostless_protocol(&lower_proto)
        && (slashes || (proto.is_some() && !is_slashed_protocol(proto.as_deref().unwrap_or(""))))
    {
        // The first `/ ? #` ends the host, but characters left of the LAST `@`
        // are auth even when they'd otherwise be illegal in a hostname:
        //   http://a@b@c/  => auth a@b, host c
        //   http://a@b?@c  => auth a,   host b, path /?@c
        let mut host_end: isize = -1;
        let mut at_sign: isize = -1;
        let mut non_host: isize = -1;
        let mut i = 0usize;
        while i < rc.len() {
            match rc[i] {
                '\t' | '\n' | '\r' => {
                    // WHATWG URL strips tab/LF/CR; so does this parser.
                    rc.remove(i);
                    continue;
                }
                ' ' | '"' | '%' | '\'' | ';' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}' => {
                    if non_host == -1 {
                        non_host = i as isize;
                    }
                }
                '#' | '/' | '?' => {
                    if non_host == -1 {
                        non_host = i as isize;
                    }
                    host_end = i as isize;
                }
                '@' => {
                    at_sign = i as isize;
                    non_host = -1;
                }
                _ => {}
            }
            if host_end != -1 {
                break;
            }
            i += 1;
        }
        let mut start = 0usize;
        if at_sign != -1 {
            u.auth = Some(super::url::percent_decode(
                &rc[..at_sign as usize].iter().collect::<String>(),
            ));
            start = at_sign as usize + 1;
        }
        if non_host == -1 {
            u.host = Some(rc[start..].iter().collect());
            rc = Vec::new();
        } else {
            u.host = Some(rc[start..non_host as usize].iter().collect());
            rc = rc[non_host as usize..].to_vec();
        }

        parse_host(&mut u);

        // The host was declared present, so `hostname` must be a string even
        // when empty.
        if u.hostname.is_none() {
            u.hostname = Some(String::new());
        }
        let hostname = u.hostname.clone().unwrap_or_default();
        let ipv6 = is_ipv6_hostname(&hostname);
        if !ipv6 {
            let rest_s: String = rc.iter().collect();
            rc = get_hostname(&mut u, &rest_s, &hostname, url)?
                .chars()
                .collect();
        }

        let hn = u.hostname.clone().unwrap_or_default();
        u.hostname = Some(if hn.chars().count() > 255 {
            String::new()
        } else {
            hn.to_lowercase()
        });

        let hn = u.hostname.clone().unwrap_or_default();
        if !hn.is_empty() {
            if ipv6 {
                if has_forbidden_host_char(&hn, true) {
                    return Err(invalid_url(url));
                }
            } else {
                // IDNA: punycode only the labels carrying non-ASCII.
                let ascii = super::punycode::to_ascii(&hn);
                u.hostname = Some(ascii.clone());
                // An empty or newly-forbidden hostname can only have come from
                // toASCII (getHostname would have split it out otherwise), so
                // this is a spoofing attempt rather than a recoverable path.
                if ascii.is_empty() || has_forbidden_host_char(&ascii, false) {
                    return Err(invalid_url(url));
                }
            }
        }

        let p = match &u.port {
            Some(p) => std::format!(":{p}"),
            None => String::new(),
        };
        let h = u.hostname.clone().unwrap_or_default();
        u.host = Some(std::format!("{h}{p}"));

        // `hostname` drops the IPv6 brackets; `host` keeps them.
        if ipv6 {
            let hn = u.hostname.clone().unwrap_or_default();
            let inner: String = {
                let c: Vec<char> = hn.chars().collect();
                if c.len() >= 2 {
                    c[1..c.len() - 1].iter().collect()
                } else {
                    String::new()
                }
            };
            u.hostname = Some(inner);
            if rc.first() != Some(&'/') {
                rc.insert(0, '/');
            }
        }
    }

    if !is_unsafe_protocol(&lower_proto) {
        rc = auto_escape_str(&rc.iter().collect::<String>())
            .chars()
            .collect();
    }

    let mut question_idx: isize = -1;
    let mut hash_idx: isize = -1;
    for (i, &c) in rc.iter().enumerate() {
        if c == '#' {
            u.hash = Some(rc[i..].iter().collect());
            hash_idx = i as isize;
            break;
        } else if c == '?' && question_idx == -1 {
            question_idx = i as isize;
        }
    }

    if question_idx != -1 {
        let q = question_idx as usize;
        if hash_idx == -1 {
            u.search = Some(rc[q..].iter().collect());
            set_query(&mut u, rc[q + 1..].iter().collect());
        } else {
            let h = hash_idx as usize;
            u.search = Some(rc[q..h].iter().collect());
            set_query(&mut u, rc[q + 1..h].iter().collect());
        }
    } else if parse_query_string {
        u.search = None;
        u.query = Some(Err(String::new()));
    }

    let use_question = question_idx != -1 && (hash_idx == -1 || question_idx < hash_idx);
    let first_idx = if use_question { question_idx } else { hash_idx };
    if first_idx == -1 {
        if !rc.is_empty() {
            u.pathname = Some(rc.iter().collect());
        }
    } else if first_idx > 0 {
        u.pathname = Some(rc[..first_idx as usize].iter().collect());
    }
    // `this.hostname` is JS-truthy here, so an EMPTY hostname (`http://?a`)
    // must NOT get the synthesized `/` pathname.
    if is_slashed_protocol(&lower_proto)
        && !u.hostname.as_deref().unwrap_or("").is_empty()
        && u.pathname.as_deref().unwrap_or("").is_empty()
    {
        u.pathname = Some("/".into());
    }

    // http.request needs `path` = pathname + search.
    if u.pathname.is_some() || u.search.is_some() {
        let p = u.pathname.clone().unwrap_or_default();
        let s = u.search.clone().unwrap_or_default();
        u.path = Some(std::format!("{p}{s}"));
    }

    u.href = Some(format_url(&u, None));
    Ok(u)
}

fn invalid_url(_url: &str) -> String {
    "TypeError [ERR_INVALID_URL]: Invalid URL".into()
}

/// The `noEscapeAuth` table: characters `Url.prototype.format` leaves as-is in
/// `auth`. Everything else is percent-encoded UTF-8.
fn auth_needs_escape(c: char) -> bool {
    !(c.is_ascii_alphanumeric() || matches!(c, '!' | '-' | '.' | '_' | '~' | '\'' | '(' | ')' | '*' | ':'))
}

fn encode_auth(auth: &str) -> String {
    let mut out = String::with_capacity(auth.len());
    for c in auth.chars() {
        if auth_needs_escape(c) {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&std::format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Port of `Url.prototype.format`. `query_string` is the already-stringified
/// `query` object (Node runs `querystring.stringify` when `query` is an object);
/// `None` means `query` was not an object.
pub fn format_url(u: &Url, query_string: Option<&str>) -> String {
    let mut auth = u.auth.clone().unwrap_or_default();
    if !auth.is_empty() {
        auth = std::format!("{}@", encode_auth(&auth));
    }

    let mut protocol = u.protocol.clone().unwrap_or_default();
    if !protocol.is_empty() && !protocol.ends_with(':') {
        protocol.push(':');
    }

    let mut pathname = u.pathname.clone().unwrap_or_default();
    let mut hash = u.hash.clone().unwrap_or_default();
    let mut host = String::new();

    if let Some(h) = u.host.as_ref().filter(|h| !h.is_empty()) {
        host = std::format!("{auth}{h}");
    } else if let Some(hn) = u.hostname.as_ref().filter(|h| !h.is_empty()) {
        let bracketed = if hn.contains(':') && !is_ipv6_hostname(hn) {
            std::format!("[{hn}]")
        } else {
            hn.clone()
        };
        host = std::format!("{auth}{bracketed}");
        if let Some(p) = u.port.as_ref().filter(|p| !p.is_empty()) {
            host.push(':');
            host.push_str(p);
        }
    }

    let query = query_string.unwrap_or("");
    let mut search = u.search.clone().unwrap_or_default();
    if search.is_empty() && !query.is_empty() {
        search = std::format!("?{query}");
    }

    if pathname.contains('#') || pathname.contains('?') {
        pathname = pathname
            .chars()
            .map(|c| match c {
                '#' => "%23".to_string(),
                '?' => "%3F".to_string(),
                c => c.to_string(),
            })
            .collect();
    }

    // Only the slashed protocols get `//`; `mailto:`/`xmpp:` keep theirs only
    // when the source had them.
    if u.slashes == Some(true) || is_slashed_protocol(&protocol) {
        if u.slashes == Some(true) || !host.is_empty() {
            if !pathname.is_empty() && !pathname.starts_with('/') {
                pathname = std::format!("/{pathname}");
            }
            host = std::format!("//{host}");
        } else if protocol.starts_with("file") {
            host = "//".into();
        }
    }

    if search.contains('#') {
        search = search.replace('#', "%23");
    }
    if !hash.is_empty() && !hash.starts_with('#') {
        hash = std::format!("#{hash}");
    }
    if !search.is_empty() && !search.starts_with('?') {
        search = std::format!("?{search}");
    }

    std::format!("{protocol}{host}{pathname}{search}{hash}")
}

// ── JS-object bridging ───────────────────────────────────────────────────────

/// Build the JS `Url`-shaped object, preserving Node's field insertion order.
pub fn to_js(u: &Url) -> Value {
    // `query` may need `querystring.parse`, which itself allocates on the host,
    // so resolve it before the `with_host` that builds the object.
    let query_val: Option<Value> = u.query.as_ref().map(|q| match q {
        Ok(raw) => with_host(|h| h.new_str(raw.clone())),
        Err(raw) => {
            let arg = with_host(|h| h.new_str(raw.clone()));
            super::querystring::call("parse", &[arg])
                .and_then(|r| r.ok())
                .unwrap_or(Value::Undef)
        }
    });
    with_host(|h| {
        let mut m = IndexMap::new();
        let opt = |h: &mut crate::host::JsHost, v: &Option<String>| match v {
            Some(s) => h.new_str(s.clone()),
            None => h.null(),
        };
        m.insert("protocol".into(), opt(h, &u.protocol));
        m.insert(
            "slashes".into(),
            match u.slashes {
                Some(b) => Value::Bool(b),
                None => h.null(),
            },
        );
        m.insert("auth".into(), opt(h, &u.auth));
        m.insert("host".into(), opt(h, &u.host));
        m.insert("port".into(), opt(h, &u.port));
        m.insert("hostname".into(), opt(h, &u.hostname));
        m.insert("hash".into(), opt(h, &u.hash));
        m.insert("search".into(), opt(h, &u.search));
        m.insert(
            "query".into(),
            query_val.clone().unwrap_or_else(|| h.null()),
        );
        m.insert("pathname".into(), opt(h, &u.pathname));
        m.insert("path".into(), opt(h, &u.path));
        m.insert("href".into(), opt(h, &u.href));
        h.new_object(m)
    })
}

/// Read a JS object back into a `Url` for `url.format(obj)`. A missing,
/// `undefined` or `null` property stays `None` (JS falsy), matching the
/// `this.x || ''` reads in `Url.prototype.format`.
fn from_js(v: &Value) -> (Url, Option<String>) {
    // `query` may be an object, which `format` stringifies via querystring.
    let query_obj = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get("query").cloned(),
        _ => None,
    });
    let query_string = match &query_obj {
        Some(q) if with_host(|h| matches!(h.get(q), Some(JsObj::Object(_)))) => {
            super::querystring::call("stringify", &[q.clone()])
                .and_then(|r| r.ok())
                .map(|s| with_host(|h| h.str_of(&s)))
        }
        _ => None,
    };
    let get = |k: &str| {
        with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => match p.get(k) {
                None | Some(Value::Undef) => None,
                Some(x) if h.is_null(x) => None,
                Some(x) => Some(h.str_of(x)),
            },
            _ => None,
        })
    };
    let slashes = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) => p.get("slashes").map(|x| h.truthy(x)),
        _ => None,
    });
    let u = Url {
        protocol: get("protocol"),
        slashes,
        auth: get("auth"),
        host: get("host"),
        port: get("port"),
        hostname: get("hostname"),
        hash: get("hash"),
        search: get("search"),
        query: None,
        pathname: get("pathname"),
        path: get("path"),
        href: get("href"),
    };
    (u, query_string)
}

/// `url.format(urlObject)` — a string is re-parsed first, a `URL` instance uses
/// its `href`, and anything else goes through `Url.prototype.format`.
pub fn format_value(v: &Value) -> Result<Value, String> {
    if let Some(s) = with_host(|h| h.as_str(v)) {
        let u = parse(&s, false, false)?;
        let out = format_url(&u, None);
        return Ok(with_host(|h| h.new_str(out)));
    }
    // A WHATWG `URL` instance formats to its `href`.
    let href = with_host(|h| match h.get(v) {
        Some(JsObj::Object(p)) if p.get("@@native").is_some() => {
            p.get("href").map(|x| h.str_of(x))
        }
        _ => None,
    });
    if let Some(href) = href {
        return Ok(with_host(|h| h.new_str(href)));
    }
    let is_obj = with_host(|h| matches!(h.get(v), Some(JsObj::Object(_))));
    if !is_obj {
        let received = super::received_desc(v);
        return Err(std::format!(
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"urlObject\" argument must be \
             one of type object or string. Received {received}"
        ));
    }
    let (u, qs) = from_js(v);
    let out = format_url(&u, qs.as_deref());
    Ok(with_host(|h| h.new_str(out)))
}
