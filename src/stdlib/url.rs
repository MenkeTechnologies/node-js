//! Node `url` module: the WHATWG `URL` class (global + `require('url').URL`) and
//! the legacy `url.parse`. A `URL` instance stores its components as data
//! properties (so `u.hostname` reads directly) plus a `@@native = "URL"` tag for
//! `toString`.

use super::arg_str;
use crate::host::{with_host, JsObj};
use fusevm::Value;
use indexmap::IndexMap;

pub const MODULE_METHODS: &[&str] = &["parse", "format"];

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
