//! JavaScript `RegExp` on top of the Rust `regex` crate.
//!
//! **Rust `regex` is NOT a superset of JavaScript's regex grammar.** It has no
//! backreferences and no lookaround, and a few escapes differ. node-js therefore
//! translates the *overlapping* subset (character classes, quantifiers, anchors,
//! groups, alternation, `\d\w\s\b`, the flags `i`/`m`/`s`) into Rust syntax and
//! **rejects at construction time** any pattern using a JS feature Rust cannot
//! express — a loud `SyntaxError`, never a silently-wrong match. The exact
//! supported subset and the known divergences are documented in BUGS.md.
//!
//! Flags: `i` (ignoreCase) and `m` (multiline) and `s` (dotAll) map onto Rust's
//! inline flags; `g` (global) and `y` (sticky) are implemented here (Rust has no
//! global flag — we drive iteration and `lastIndex` ourselves); `u`/`d` are
//! accepted and otherwise ignored.

use crate::host::{self, with_host, JsObj, RegExpObj};
use fusevm::Value;
use indexmap::IndexMap;

/// Build a `RegExp` value from a JS `pattern` + `flags`, or a `SyntaxError` if the
/// pattern uses an unsupported construct or is otherwise invalid.
pub fn build_regexp(pattern: &str, flags: &str) -> Result<Value, String> {
    // Validate flags (Node throws on an unknown/repeated flag).
    let mut seen = String::new();
    for c in flags.chars() {
        if !"gimsuyd".contains(c) || seen.contains(c) {
            return Err(format!("SyntaxError: Invalid flags supplied to RegExp constructor '{flags}'"));
        }
        seen.push(c);
    }
    let global = flags.contains('g');
    let ignore_case = flags.contains('i');
    let multiline = flags.contains('m');
    let dot_all = flags.contains('s');
    let sticky = flags.contains('y');
    let unicode = flags.contains('u');

    let rust_pat = translate(pattern)?;
    // Assemble the inline-flag prefix Rust understands.
    let mut prefixed = String::new();
    if ignore_case || multiline || dot_all {
        prefixed.push_str("(?");
        if ignore_case {
            prefixed.push('i');
        }
        if multiline {
            prefixed.push('m');
        }
        if dot_all {
            prefixed.push('s');
        }
        prefixed.push(')');
    }
    prefixed.push_str(&rust_pat);

    let re = regex::Regex::new(&prefixed).map_err(|e| {
        // Collapse Rust's multi-line error to one line for a JS-shaped message.
        let msg = e.to_string().lines().collect::<Vec<_>>().join(" ");
        format!("SyntaxError: Invalid regular expression: /{pattern}/: {msg}")
    })?;

    let obj = RegExpObj {
        re,
        source: if pattern.is_empty() { "(?:)".to_string() } else { pattern.to_string() },
        flags: flags.to_string(),
        global,
        ignore_case,
        multiline,
        dot_all,
        sticky,
        unicode,
        last_index: 0,
    };
    Ok(with_host(|h| h.alloc(JsObj::RegExp(Box::new(obj)))))
}

/// Translate a JS regex source into Rust `regex` syntax, rejecting constructs Rust
/// cannot represent. Returns a `SyntaxError` string for the unsupported cases.
fn translate(pat: &str) -> Result<String, String> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let n = chars.get(i + 1).copied();
                match n {
                    // Backreference `\1`..`\9` (outside a class) — unsupported.
                    Some(d) if d.is_ascii_digit() && d != '0' && !in_class => {
                        return Err(unsupported("backreferences"));
                    }
                    // Named backreference `\k<name>` — unsupported.
                    Some('k') if !in_class => return Err(unsupported("named backreferences")),
                    // `\uXXXX` / `\u{...}` → Rust `\x{...}`.
                    Some('u') => {
                        i += 2;
                        if chars.get(i) == Some(&'{') {
                            out.push_str("\\x{");
                            i += 1;
                            while i < chars.len() && chars[i] != '}' {
                                out.push(chars[i]);
                                i += 1;
                            }
                            out.push('}');
                            i += 1; // consume '}'
                        } else {
                            // Exactly four hex digits.
                            let hex: String = chars[i..(i + 4).min(chars.len())].iter().collect();
                            out.push_str(&format!("\\x{{{hex}}}"));
                            i += 4;
                        }
                        continue;
                    }
                    // `\/` in a JS literal → a plain slash (Rust rejects `\/`).
                    Some('/') => {
                        out.push('/');
                        i += 2;
                        continue;
                    }
                    // Everything else (`\d \w \s \b \n \. \\` …) passes through.
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                        i += 2;
                        continue;
                    }
                    None => {
                        out.push('\\');
                        i += 1;
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push('[');
                i += 1;
            }
            ']' if in_class => {
                in_class = false;
                out.push(']');
                i += 1;
            }
            '(' if !in_class => {
                // Distinguish `(?<name>` (named group → `(?P<name>`) from
                // `(?<=` / `(?<!` (lookbehind → unsupported) and `(?=`/`(?!`
                // (lookahead → unsupported).
                if chars.get(i + 1) == Some(&'?') {
                    match chars.get(i + 2) {
                        Some('=') | Some('!') => return Err(unsupported("lookahead assertions")),
                        Some('<') => match chars.get(i + 3) {
                            Some('=') | Some('!') => {
                                return Err(unsupported("lookbehind assertions"))
                            }
                            _ => {
                                out.push_str("(?P<");
                                i += 3;
                                continue;
                            }
                        },
                        _ => {
                            out.push('(');
                            i += 1;
                        }
                    }
                } else {
                    out.push('(');
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn unsupported(feature: &str) -> String {
    format!(
        "SyntaxError: node-js regex does not support {feature} (Rust `regex` backend); \
         see BUGS.md for the supported subset"
    )
}

/// A `RegExp` own data property (`source`/`flags`/`global`/…/`lastIndex`), or
/// `None` if `name` is not one (so the caller tries methods).
pub fn regexp_property(r: &RegExpObj, name: &str) -> Option<Value> {
    Some(match name {
        "source" => with_host(|h| h.new_str(r.source.clone())),
        "flags" => with_host(|h| h.new_str(r.flags.clone())),
        "global" => Value::Bool(r.global),
        "ignoreCase" => Value::Bool(r.ignore_case),
        "multiline" => Value::Bool(r.multiline),
        "dotAll" => Value::Bool(r.dot_all),
        "sticky" => Value::Bool(r.sticky),
        "unicode" => Value::Bool(r.unicode),
        "lastIndex" => Value::Float(r.last_index as f64),
        _ => return None,
    })
}

pub fn is_regexp_method(name: &str) -> bool {
    matches!(name, "test" | "exec" | "toString" | "compile")
}

/// Dispatch a `RegExp.prototype` method.
pub fn regexp_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "test" => {
            let s = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            Ok(Value::Bool(regexp_test(recv, &s)))
        }
        "exec" => {
            let s = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            regexp_exec(recv, &s)
        }
        "toString" => Ok(with_host(|h| {
            let s = h.str_of(recv);
            h.new_str(s)
        })),
        // `compile` is a legacy no-op here (the pattern is already compiled).
        "compile" => Ok(recv.clone()),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// Snapshot the fields we need without holding the host borrow across a match.
fn regexp_snapshot(recv: &Value) -> Option<(regex::Regex, bool, bool, usize)> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::RegExp(r)) => Some((r.re.clone(), r.global, r.sticky, r.last_index)),
        _ => None,
    })
}

fn set_last_index(recv: &Value, idx: usize) {
    with_host(|h| {
        if let Some(JsObj::RegExp(r)) = h.get_mut(recv) {
            r.last_index = idx;
        }
    });
}

/// Byte offset of the `n`-th char (clamped to the string length).
fn byte_of_char(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(b, _)| b).unwrap_or(s.len())
}
/// Char index of a byte offset.
fn char_of_byte(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

/// `re.test(s)` — honoring `g`/`y` `lastIndex` advancement, exactly like `exec`.
pub fn regexp_test(recv: &Value, s: &str) -> bool {
    let Some((re, global, sticky, last)) = regexp_snapshot(recv) else {
        return false;
    };
    let start_char = if global || sticky { last } else { 0 };
    if start_char > s.chars().count() {
        if global || sticky {
            set_last_index(recv, 0);
        }
        return false;
    }
    let start_byte = byte_of_char(s, start_char);
    match re.find_at(s, start_byte) {
        Some(m) if !sticky || m.start() == start_byte => {
            if global || sticky {
                set_last_index(recv, char_of_byte(s, m.end()));
            }
            true
        }
        _ => {
            if global || sticky {
                set_last_index(recv, 0);
            }
            false
        }
    }
}

/// `re.exec(s)` — returns a match array (`[full, ...captures]` with `.index`,
/// `.input`, `.groups`), or `null`. Advances `lastIndex` under `g`/`y`.
pub fn regexp_exec(recv: &Value, s: &str) -> Result<Value, String> {
    let Some((re, global, sticky, last)) = regexp_snapshot(recv) else {
        return Ok(with_host(|h| h.null()));
    };
    let start_char = if global || sticky { last } else { 0 };
    if start_char > s.chars().count() {
        if global || sticky {
            set_last_index(recv, 0);
        }
        return Ok(with_host(|h| h.null()));
    }
    let start_byte = byte_of_char(s, start_char);
    let caps = re.captures_at(s, start_byte);
    let caps = match caps {
        Some(c) if !sticky || c.get(0).map(|m| m.start()) == Some(start_byte) => c,
        _ => {
            if global || sticky {
                set_last_index(recv, 0);
            }
            return Ok(with_host(|h| h.null()));
        }
    };
    let whole = caps.get(0).unwrap();
    if global || sticky {
        set_last_index(recv, char_of_byte(s, whole.end()));
    }
    Ok(build_match_array(&re, &caps, s))
}

/// Build the JS match-result array from a `Captures`, attaching `.index`,
/// `.input`, and (named-group) `.groups`.
fn build_match_array(re: &regex::Regex, caps: &regex::Captures, s: &str) -> Value {
    let mut items: Vec<Value> = Vec::with_capacity(caps.len());
    for i in 0..caps.len() {
        items.push(match caps.get(i) {
            Some(m) => with_host(|h| h.new_str(m.as_str().to_string())),
            None => Value::Undef, // a non-participating optional group
        });
    }
    let whole = caps.get(0).unwrap();
    let arr = with_host(|h| h.new_array(items));
    let index = char_of_byte(s, whole.start());
    with_host(|h| {
        let idx = Value::Float(index as f64);
        h.set_fn_prop(&arr, "index", idx);
        let input = h.new_str(s.to_string());
        h.set_fn_prop(&arr, "input", input);
    });
    // Named groups → a `.groups` object (or `undefined` if the regex has none).
    let names: Vec<&str> = re.capture_names().flatten().collect();
    if names.is_empty() {
        with_host(|h| h.set_fn_prop(&arr, "groups", Value::Undef));
    } else {
        let mut g: IndexMap<String, Value> = IndexMap::new();
        for name in names {
            let v = match caps.name(name) {
                Some(m) => with_host(|h| h.new_str(m.as_str().to_string())),
                None => Value::Undef,
            };
            g.insert(name.to_string(), v);
        }
        with_host(|h| {
            let obj = h.new_object(g);
            h.set_fn_prop(&arr, "groups", obj);
        });
    }
    arr
}

// ── String.prototype regex methods (called from builtins::string_method) ──────

/// `str.match(re)`: without `g`, same as `exec` (array or null); with `g`, an
/// array of every whole-match string (or null if none).
pub fn str_match(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, global, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.null()));
    };
    if !global {
        // Non-global match ignores lastIndex and searches from the start.
        set_last_index(re_val, 0);
        return regexp_exec_from_zero(&re, s);
    }
    let matches: Vec<Value> = re
        .find_iter(s)
        .map(|m| with_host(|h| h.new_str(m.as_str().to_string())))
        .collect();
    if matches.is_empty() {
        Ok(with_host(|h| h.null()))
    } else {
        Ok(with_host(|h| h.new_array(matches)))
    }
}

/// Non-global exec searching from offset 0 (for `str.match` without `g`).
fn regexp_exec_from_zero(re: &regex::Regex, s: &str) -> Result<Value, String> {
    match re.captures(s) {
        Some(caps) => Ok(build_match_array(re, &caps, s)),
        None => Ok(with_host(|h| h.null())),
    }
}

/// `str.matchAll(re)`: an iterator over every match array (requires the `g` flag
/// in Node, but we accept a non-global regex too and still iterate all matches).
pub fn str_match_all(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, _, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    };
    let mut items = Vec::new();
    for caps in re.captures_iter(s) {
        items.push(build_match_array(&re, &caps, s));
    }
    // Return a live iterator so `for-of`/spread/`Array.from` all work.
    Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
}

/// `str.search(re)`: char index of the first match, or -1.
pub fn str_search(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, _, _, _)) = regexp_snapshot(re_val) else {
        return Ok(Value::Float(-1.0));
    };
    Ok(match re.find(s) {
        Some(m) => Value::Float(char_of_byte(s, m.start()) as f64),
        None => Value::Float(-1.0),
    })
}

/// `str.split(re[, limit])`: split on regex matches; captured groups are spliced
/// into the output (JS semantics).
pub fn str_split_regex(s: &str, re_val: &Value, limit: Option<usize>) -> Result<Value, String> {
    let Some((re, _, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    };
    let mut out: Vec<Value> = Vec::new();
    let mut last_end = 0usize;
    for caps in re.captures_iter(s) {
        let m = caps.get(0).unwrap();
        // Zero-width match at the very start is skipped (matches JS closely).
        if m.start() == m.end() && m.start() == last_end && last_end == 0 {
            continue;
        }
        out.push(with_host(|h| h.new_str(s[last_end..m.start()].to_string())));
        // Splice in captured groups (1..).
        for i in 1..caps.len() {
            out.push(match caps.get(i) {
                Some(g) => with_host(|h| h.new_str(g.as_str().to_string())),
                None => Value::Undef,
            });
        }
        last_end = m.end();
        if let Some(l) = limit {
            if out.len() >= l {
                out.truncate(l);
                return Ok(with_host(|h| h.new_array(out)));
            }
        }
    }
    out.push(with_host(|h| h.new_str(s[last_end..].to_string())));
    if let Some(l) = limit {
        out.truncate(l);
    }
    Ok(with_host(|h| h.new_array(out)))
}

/// `str.replace(re, repl)` / `str.replaceAll(re, repl)`. `repl` is either a string
/// (with `$1`/`$&`/`` $` ``/`$'`/`$<name>`/`$$` patterns) or a function replacer.
pub fn str_replace_regex(s: &str, re_val: &Value, repl: &Value, all: bool) -> Result<Value, String> {
    let Some((re, global, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_str(s.to_string())));
    };
    let replace_all = all || global;
    let is_fn = with_host(|h| host::is_callable(h, repl));

    let mut out = String::new();
    let mut last = 0usize;
    let mut count = 0;
    for caps in re.captures_iter(s) {
        let m = caps.get(0).unwrap();
        out.push_str(&s[last..m.start()]);
        if is_fn {
            // fn(match, p1, …, offset, whole_string)
            let mut call_args: Vec<Value> = Vec::new();
            for i in 0..caps.len() {
                call_args.push(match caps.get(i) {
                    Some(g) => with_host(|h| h.new_str(g.as_str().to_string())),
                    None => Value::Undef,
                });
            }
            call_args.push(Value::Float(char_of_byte(s, m.start()) as f64));
            call_args.push(with_host(|h| h.new_str(s.to_string())));
            let r = host::invoke(repl, call_args, None)?;
            out.push_str(&with_host(|h| h.str_of(&r)));
        } else {
            let repl_str = with_host(|h| h.str_of(repl));
            out.push_str(&expand_replacement(&repl_str, &caps, s));
        }
        last = m.end();
        count += 1;
        if !replace_all && count >= 1 {
            break;
        }
    }
    out.push_str(&s[last..]);
    Ok(with_host(|h| h.new_str(out)))
}

/// Expand a replacement template's `$` patterns against a match.
fn expand_replacement(templ: &str, caps: &regex::Captures, s: &str) -> String {
    let chars: Vec<char> = templ.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let whole = caps.get(0).unwrap();
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                '$' => {
                    out.push('$');
                    i += 2;
                }
                '&' => {
                    out.push_str(whole.as_str());
                    i += 2;
                }
                '`' => {
                    out.push_str(&s[..whole.start()]);
                    i += 2;
                }
                '\'' => {
                    out.push_str(&s[whole.end()..]);
                    i += 2;
                }
                '<' => {
                    // `$<name>` named-group reference.
                    let mut j = i + 2;
                    let mut name = String::new();
                    while j < chars.len() && chars[j] != '>' {
                        name.push(chars[j]);
                        j += 1;
                    }
                    if let Some(m) = caps.name(&name) {
                        out.push_str(m.as_str());
                    }
                    i = j + 1; // consume '>'
                }
                d if d.is_ascii_digit() => {
                    // `$1`..`$99`: prefer a two-digit group if it exists.
                    let d2 = chars.get(i + 2).copied().filter(|c| c.is_ascii_digit());
                    let two = d2.and_then(|c2| format!("{d}{c2}").parse::<usize>().ok());
                    if let Some(gi) = two.filter(|gi| *gi < caps.len()) {
                        if let Some(g) = caps.get(gi) {
                            out.push_str(g.as_str());
                        }
                        i += 3;
                    } else {
                        let gi = d.to_digit(10).unwrap() as usize;
                        if gi >= 1 && gi < caps.len() {
                            if let Some(g) = caps.get(gi) {
                                out.push_str(g.as_str());
                            }
                            i += 2;
                        } else {
                            out.push('$');
                            i += 1;
                        }
                    }
                }
                _ => {
                    out.push('$');
                    i += 1;
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}
