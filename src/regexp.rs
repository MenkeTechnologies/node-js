//! JavaScript `RegExp` on top of the [`fancy_regex`] crate.
//!
//! `fancy-regex` wraps the linear Rust `regex` engine and layers a backtracking
//! matcher on top, so it can express the JS constructs plain `regex` cannot:
//! lookahead (`(?=)`/`(?!)`), lookbehind (`(?<=)`/`(?<!)`), and backreferences
//! (`\1`, `\k<name>`). node-js therefore accepts a near-superset of the JS regex
//! grammar; the small residue fancy-regex still cannot represent is documented
//! in BUGS.md and rejected loudly at construction time (never a silently-wrong
//! match).
//!
//! What `translate` still has to do (fancy-regex/regex differ from JS here):
//!   * `\uXXXX` / `\u{...}` → `\x{...}` (regex spells fixed code points that
//!     way), with lone-surrogate escapes (`\uD800`..`\uDFFF`) mapped into a
//!     Plane-15 private-use block — surrogate code points are not valid Unicode
//!     scalar values, so `\x{D800}` will not compile; a valid UTF-8 `&str` can
//!     never contain a lone surrogate anyway, so those alternatives stay dead
//!     (correct for all valid input, e.g. encodeurl's unmatched-surrogate scan).
//!   * `\/` in a literal → a plain `/` (regex rejects the redundant escape).
//!
//! Everything else — including `(?<name>...)`, `(?=)`/`(?!)`, `(?<=)`/`(?<!)`,
//! and `\1`/`\k<name>` — passes through verbatim; fancy-regex parses it natively.
//!
//! Flags: `i`/`m`/`s` map onto inline flags; `g`/`y` drive iteration and
//! `lastIndex` here (fancy-regex has no global flag); `u`/`d` are accepted.

use crate::host::{self, with_host, JsObj, RegExpObj};
use crate::utf16::{self, U16Index};
use fancy_regex::{Captures, Regex};
use fusevm::Value;
use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

/// Lone-surrogate code points are not valid Unicode scalar values, so they can
/// never appear in a Rust `&str` and `regex` refuses to compile `\x{D800}`. Map
/// the 2048-code-point surrogate block bijectively into Plane-15 PUA-B, which is
/// valid, contiguous (so class ranges stay ranges), and never occurs in normal
/// text — the surrogate alternatives thus compile and stay inert on valid input.
const SURROGATE_LO: u32 = 0xD800;
const SURROGATE_HI: u32 = 0xDFFF;
const SURROGATE_PUA_BASE: u32 = 0xF_0000;

/// Remap a surrogate code point into the inert PUA block; pass others through.
fn remap_surrogate(cp: u32) -> u32 {
    if (SURROGATE_LO..=SURROGATE_HI).contains(&cp) {
        SURROGATE_PUA_BASE + (cp - SURROGATE_LO)
    } else {
        cp
    }
}

/// Build a `RegExp` value from a JS `pattern` + `flags`, or a `SyntaxError` if the
/// pattern uses an unsupported construct or is otherwise invalid.
/// 22.2.6.13.1 EscapeRegExpPattern — the text `source` reports, escaped so that
/// `/` + source + `/` is a parseable regular-expression literal that matches the
/// same thing.
///
/// Two characters need it and neither was handled: an unescaped `/` ended the
/// literal early, so `String(new RegExp('/'))` produced `///`, and a literal
/// line terminator cannot appear in a literal at all, so `new RegExp('\n')`
/// reported a raw newline where node reports the two characters `\n`.
///
/// A `/` inside a character class does NOT need escaping and node does not add
/// one (`new RegExp('[/]').source` is `[/]`), so the scan tracks class depth.
/// An already-escaped `\/` is left alone rather than doubled.
fn escape_regexp_pattern(pattern: &str) -> String {
    if pattern.is_empty() {
        return "(?:)".to_string();
    }
    let mut out = String::with_capacity(pattern.len());
    let mut in_class = false;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            // A backslash escapes whatever follows; both travel through as-is.
            '\\' => {
                out.push(c);
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            '/' if !in_class => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

pub fn build_regexp(pattern: &str, flags: &str) -> Result<Value, String> {
    // Validate flags (Node throws on an unknown/repeated flag).
    let mut seen = String::new();
    for c in flags.chars() {
        if !"gimsuyd".contains(c) || seen.contains(c) {
            return Err(format!(
                "SyntaxError: Invalid flags supplied to RegExp constructor '{flags}'"
            ));
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
    // Assemble the inline-flag prefix fancy-regex (via the regex layer) understands.
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

    let re = compiled(&prefixed).map_err(|e| {
        // Collapse the multi-line error to one line for a JS-shaped message.
        let msg = e.lines().collect::<Vec<_>>().join(" ");
        format!("SyntaxError: Invalid regular expression: /{pattern}/: {msg}")
    })?;

    // A `RegExpObj` is unavoidably fresh per evaluation (`lastIndex` is
    // per-object mutable state), but the engine inside it is not.
    let obj = RegExpObj {
        re,
        source: escape_regexp_pattern(pattern),
        flags: flags.to_string(),
        global,
        ignore_case,
        multiline,
        dot_all,
        sticky,
        unicode,
        last_index: U16Index::ZERO,
    };
    Ok(with_host(|h| h.alloc(JsObj::RegExp(Box::new(obj)))))
}

/// The compiled engine for an already-translated, flag-prefixed pattern,
/// compiling it at most once per process.
///
/// A JS regex LITERAL is re-evaluated every time control reaches it, and each
/// evaluation must produce a fresh `RegExp` object (`lastIndex` is per-object
/// mutable state). Building the ENGINE each time as well is what made module
/// loading slow: `require("express")` compiled 1,782 regexes drawn from only 59
/// distinct patterns, and `fancy_regex::Regex::new` — not matching — accounted
/// for 85% of the wall time. A single literal inside a hot function is the worst
/// case: `mime-types`' `/(\.|x-).*/` cost ~1.5 ms per compile, so 2,582 loop
/// iterations spent 4.2 s compiling one constant pattern. Hoisting that same
/// regex out of the loop by hand took it to 27 ms, which is what identified
/// compilation rather than matching as the cost.
///
/// Keyed on the translated + prefixed pattern, so two literals that differ only
/// in spelling before translation still share one engine, and two that differ in
/// flags do not. A compile FAILURE is not cached: it is a one-off cost on a path
/// that immediately throws, and caching it would mean holding the error string
/// for the life of the process.
///
/// Unbounded on purpose. The entries are the distinct regexes a program's source
/// contains, which is a property of the code rather than of the input — the 59
/// above is what a whole express dependency tree amounts to. A program that
/// builds patterns from unbounded INPUT (`new RegExp(userString)`) is the case
/// this would grow with, and it is also the case that gets no benefit; if that
/// ever matters the fix is a capacity bound here, not a different design.
fn compiled(prefixed: &str) -> Result<Rc<Regex>, String> {
    thread_local! {
        static CACHE: RefCell<FxHashMap<String, Rc<Regex>>> =
            RefCell::new(FxHashMap::default());
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(prefixed).cloned()) {
        return Ok(hit);
    }
    let re = Rc::new(Regex::new(prefixed).map_err(|e| e.to_string())?);
    CACHE.with(|c| {
        c.borrow_mut().insert(prefixed.to_string(), re.clone());
    });
    Ok(re)
}

/// Translate a JS regex source into fancy-regex syntax. The rewrites needed are
/// the `\u`→`\x{}` code-point spelling (with surrogate remapping), the redundant
/// `\/` escape, and escaping a bare `[` inside a character class — JS treats it
/// as a literal, but the `regex` layer parses it as a (nested) class open and
/// errors ("Invalid character class"). lookaround/backrefs/named groups pass
/// through verbatim.
fn translate(pat: &str) -> Result<String, String> {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    // Track whether we're inside a `[...]` class. `class_pos` is how many chars
    // into the current class we are, so we can spot the `]` that would close an
    // empty class (`[]` / `[^]`) vs. a literal leading `]`.
    let mut in_class = false;
    let mut class_pos = 0usize;
    while i < chars.len() {
        let c = chars[i];
        // Character-class bookkeeping. A `\` escape is handled below and never
        // toggles class state (it consumes its own two chars).
        if c != '\\' {
            if !in_class && c == '[' {
                in_class = true;
                class_pos = 0;
                out.push('[');
                i += 1;
                // A leading `^` is the negation, not the first member.
                if chars.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                continue;
            }
            if in_class {
                // The first char of a class, if `]`, is a literal `]` in JS; a
                // later bare `[` must be escaped for the regex layer.
                if c == ']' && class_pos > 0 {
                    in_class = false;
                    out.push(']');
                    i += 1;
                    continue;
                }
                if c == '[' {
                    out.push_str("\\[");
                    class_pos += 1;
                    i += 1;
                    continue;
                }
            }
        }
        match c {
            '\\' => {
                class_pos += 1;
                match chars.get(i + 1).copied() {
                    // `\uXXXX` / `\u{...}` → `\x{...}` (surrogates remapped).
                    Some('u') => {
                        i += 2;
                        let cp_hex: String;
                        if chars.get(i) == Some(&'{') {
                            i += 1;
                            let mut hex = String::new();
                            while i < chars.len() && chars[i] != '}' {
                                hex.push(chars[i]);
                                i += 1;
                            }
                            i += 1; // consume '}'
                            cp_hex = hex;
                        } else {
                            // Exactly four hex digits.
                            cp_hex = chars[i..(i + 4).min(chars.len())].iter().collect();
                            i += 4;
                        }
                        match u32::from_str_radix(cp_hex.trim(), 16) {
                            Ok(cp) => out.push_str(&format!("\\x{{{:X}}}", remap_surrogate(cp))),
                            // Not valid hex — emit the code point literally so the
                            // engine surfaces its own error rather than us guessing.
                            Err(_) => out.push_str(&format!("\\x{{{cp_hex}}}")),
                        }
                        continue;
                    }
                    // `\/` in a JS literal → a plain slash (regex rejects `\/`).
                    Some('/') => {
                        out.push('/');
                        i += 2;
                        continue;
                    }
                    // Everything else (`\d \w \s \b \1 \k \n \. \\` …) passes through.
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
            _ => {
                if in_class {
                    class_pos += 1;
                }
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// The flags string in the spec's canonical order (22.2.6.4 reads the six
/// reflectors in a fixed sequence), independent of how the literal spelled
/// them.
fn canonical_flags(flags: &str) -> String {
    "dgimsuvy"
        .chars()
        .filter(|c| flags.contains(*c))
        .collect::<String>()
}

/// A `RegExp` own data property (`source`/`flags`/`global`/…/`lastIndex`), or
/// `None` if `name` is not one (so the caller tries methods).
pub fn regexp_property(r: &RegExpObj, name: &str) -> Option<Value> {
    Some(match name {
        "source" => with_host(|h| h.new_str(r.source.clone())),
        // `RegExp.prototype.flags` (22.2.6.4) is a GETTER that rebuilds the
        // string in the spec's fixed `dgimsuvy` order, not the spelling the
        // literal used: `/a/gid.flags` is `"dgi"` in node and was `"gid"` here,
        // so any code keyed on the flags string (a cache key, a `new
        // RegExp(src, flags)` round-trip comparison) disagreed.
        "flags" => with_host(|h| h.new_str(canonical_flags(&r.flags))),
        "global" => Value::Bool(r.global),
        "ignoreCase" => Value::Bool(r.ignore_case),
        "multiline" => Value::Bool(r.multiline),
        "dotAll" => Value::Bool(r.dot_all),
        "sticky" => Value::Bool(r.sticky),
        "unicode" => Value::Bool(r.unicode),
        // `d` is accepted and its match-indices output ignored (BUGS.md), but
        // the flag reflector still has to report it; it read `undefined` where
        // node says `true`/`false`. `v` is rejected at construction time, so
        // `unicodeSets` is `false` for every regex that exists here.
        "hasIndices" => Value::Bool(r.flags.contains('d')),
        "unicodeSets" => Value::Bool(r.flags.contains('v')),
        "lastIndex" => Value::Float(r.last_index.get() as f64),
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
fn regexp_snapshot(recv: &Value) -> Option<(Rc<Regex>, bool, bool, U16Index)> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::RegExp(r)) => Some((r.re.clone(), r.global, r.sticky, r.last_index)),
        _ => None,
    })
}

fn set_last_index(recv: &Value, idx: U16Index) {
    with_host(|h| {
        if let Some(JsObj::RegExp(r)) = h.get_mut(recv) {
            r.last_index = idx;
        }
    });
}

/// Byte offset of a UTF-16 index (clamped to the string length).
///
/// `lastIndex` and `.index` are UTF-16 code-unit offsets in JS, while the regex
/// engine works in UTF-8 byte offsets. Both are `usize`-shaped, so the newtype
/// is what stops one being passed where the other belongs.
fn byte_of_index(s: &str, n: U16Index) -> usize {
    utf16::byte_of_index(s, n)
}
/// UTF-16 index of a byte offset.
fn index_of_byte(s: &str, byte: usize) -> U16Index {
    utf16::index_of_byte(s, byte)
}

/// `re.test(s)` — honoring `g`/`y` `lastIndex` advancement, exactly like `exec`.
pub fn regexp_test(recv: &Value, s: &str) -> bool {
    let Some((re, global, sticky, last)) = regexp_snapshot(recv) else {
        return false;
    };
    let start_idx = if global || sticky {
        last
    } else {
        U16Index::ZERO
    };
    if start_idx.get() > utf16::len(s) {
        if global || sticky {
            set_last_index(recv, U16Index::ZERO);
        }
        return false;
    }
    let start_byte = byte_of_index(s, start_idx);
    // A backtracking match can fail (catastrophic backtracking guard); treat an
    // engine error as "no match" so a pathological pattern never panics the VM.
    match re.find_from_pos(s, start_byte) {
        Ok(Some(m)) if !sticky || m.start() == start_byte => {
            if global || sticky {
                set_last_index(recv, index_of_byte(s, m.end()));
            }
            true
        }
        _ => {
            if global || sticky {
                set_last_index(recv, U16Index::ZERO);
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
    let start_idx = if global || sticky {
        last
    } else {
        U16Index::ZERO
    };
    if start_idx.get() > utf16::len(s) {
        if global || sticky {
            set_last_index(recv, U16Index::ZERO);
        }
        return Ok(with_host(|h| h.null()));
    }
    let start_byte = byte_of_index(s, start_idx);
    let caps = re.captures_from_pos(s, start_byte).ok().flatten();
    let caps = match caps {
        Some(c) if !sticky || c.get(0).map(|m| m.start()) == Some(start_byte) => c,
        _ => {
            if global || sticky {
                set_last_index(recv, U16Index::ZERO);
            }
            return Ok(with_host(|h| h.null()));
        }
    };
    let whole = caps.get(0).unwrap();
    if global || sticky {
        set_last_index(recv, index_of_byte(s, whole.end()));
    }
    Ok(build_match_array(&re, &caps, s))
}

/// Build the JS match-result array from a `Captures`, attaching `.index`,
/// `.input`, and (named-group) `.groups`.
fn build_match_array(re: &Regex, caps: &Captures, s: &str) -> Value {
    let mut items: Vec<Value> = Vec::with_capacity(caps.len());
    for i in 0..caps.len() {
        items.push(match caps.get(i) {
            Some(m) => with_host(|h| h.new_str(m.as_str().to_string())),
            None => Value::Undef, // a non-participating optional group
        });
    }
    let whole = caps.get(0).unwrap();
    let arr = with_host(|h| h.new_array(items));
    let index = index_of_byte(s, whole.start()).get();
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
            // `groups` is an `OrdinaryObjectCreate(null)` (22.2.7.2 step 30), so
            // it inherits nothing and inspects as `[Object: null prototype]`.
            let null = h.null();
            h.set_proto(&obj, null);
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
        set_last_index(re_val, U16Index::ZERO);
        return regexp_exec_from_zero(&re, s);
    }
    // 22.2.6.9 step 6.a: a global match sets `lastIndex` to 0 before it starts,
    // so it always collects from the beginning and leaves it there. It was
    // being left wherever the caller had put it.
    set_last_index(re_val, U16Index::ZERO);
    let matches: Vec<Value> = re
        .find_iter(s)
        .filter_map(|m| m.ok())
        .map(|m| with_host(|h| h.new_str(m.as_str().to_string())))
        .collect();
    if matches.is_empty() {
        Ok(with_host(|h| h.null()))
    } else {
        Ok(with_host(|h| h.new_array(matches)))
    }
}

/// Non-global exec searching from offset 0 (for `str.match` without `g`).
fn regexp_exec_from_zero(re: &Regex, s: &str) -> Result<Value, String> {
    match re.captures(s).ok().flatten() {
        Some(caps) => Ok(build_match_array(re, &caps, s)),
        None => Ok(with_host(|h| h.null())),
    }
}

/// `str.matchAll(re)`: an iterator over every match array (requires the `g` flag
/// in Node, but we accept a non-global regex too and still iterate all matches).
pub fn str_match_all(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, global, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    };
    // 22.1.3.14 step 5.c: a non-global regexp is a TypeError, because
    // `matchAll` cannot produce every match without `g` — the same rule
    // `replaceAll` enforces. This used to return just the first match.
    if !global {
        return Err(host::type_error(
            "String.prototype.matchAll called with a non-global RegExp argument",
        ));
    }
    let mut items = Vec::new();
    for caps in re.captures_iter(s).flatten() {
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
    Ok(match re.find(s).ok().flatten() {
        Some(m) => Value::Float(index_of_byte(s, m.start()).get() as f64),
        None => Value::Float(-1.0),
    })
}

/// `str.split(re[, limit])`: split on regex matches; captured groups are spliced
/// into the output (JS semantics).
/// `String.prototype.split(regexp[, limit])` — 22.2.6.14 `RegExp.prototype
/// [@@split]`.
///
/// The previous shape of this was a `captures_iter` with one hand-rolled
/// special case for a zero-width match at position 0, and it got every other
/// empty-match position wrong: `'ab'.split(/(?:)/)` grew a trailing `""`,
/// `''.split(/(?:)/)` answered `[""]` instead of `[]`, and `'ab'.split(/()/)`
/// answered five elements instead of three.
///
/// The spec loop is what gets those right, and the single rule doing the work
/// is `e == p`: a match ENDING where the previous piece began contributes
/// nothing and only advances the scan. The final piece is always the tail from
/// `p`, appended after the loop — which is why an empty match at the very end
/// does not produce an extra `""` (the loop stops at `q == size` before ever
/// matching there) while a real separator at the end does.
///
/// The scan is in UTF-16 code units, since that is what the spec indexes and
/// what `.index`/`lastIndex` report elsewhere in this file. Where the spec
/// advances `q` one position at a time until a match occurs AT `q`, this jumps
/// straight to the next match at or after `q`: every position skipped is one
/// the spec would have failed to match, so the two agree.
///
/// One divergence remains and is not fixable here: a lone surrogate cannot be
/// represented in a Rust `String`, so a split position INSIDE an astral
/// character does not exist to be split at. `'\u{1F600}a'.split(/(?:)/)` is
/// `['\u{1F600}', 'a']` here and three elements in node, which splits the
/// surrogate pair. `'\u{1F600}'.split('')` has always had the same limit; it
/// is the string representation, not this algorithm.
pub fn str_split_regex(s: &str, re_val: &Value, limit: Option<usize>) -> Result<Value, String> {
    let Some((re, _, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    };
    let lim = limit.unwrap_or(usize::MAX);
    if lim == 0 {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    }
    let size = utf16::len(s);
    let units = |i: usize| byte_of_index(s, U16Index::new(i));

    // An empty subject splits to nothing when the separator can match it, and
    // to `[""]`... which is the subject itself, when it cannot.
    if size == 0 {
        let out = if matches!(re.find(s), Ok(Some(_))) {
            Vec::new()
        } else {
            vec![with_host(|h| h.new_str(String::new()))]
        };
        return Ok(with_host(|h| h.new_array(out)));
    }

    let mut out: Vec<Value> = Vec::new();
    let mut p = 0usize; // start of the piece being accumulated
    let mut q = 0usize; // scan position
    while q < size {
        let Some(caps) = re.captures_from_pos(s, units(q)).ok().flatten() else {
            break;
        };
        let m = caps.get(0).expect("group 0 always participates");
        let m_start = index_of_byte(s, m.start()).get();
        // The spec scans `q` only while `q < size` and requires the match to be
        // AT `q`, so a match starting at the very end is never reached. Jumping
        // to the next match does reach it, and letting it through appended a
        // spurious trailing `""` for every end-anchored zero-width separator —
        // `/$/`, `/\b/`, a trailing lookbehind.
        if m_start >= size {
            break;
        }
        let e = index_of_byte(s, m.end()).get().min(size);
        if e == p {
            // Contributes no piece; step past this position and rescan.
            //
            // The step is off `q`, not off `m_start`, because the two can move
            // backwards relative to each other: a unit index that falls INSIDE
            // an astral character has no byte offset of its own, so the search
            // starts at the character's first byte and reports a match before
            // `q`. Stepping off `m_start` there left `q` pinned and the loop
            // spun forever on `'\u{1F600}a'.split(/(?:)/)`.
            q = m_start.max(q) + 1;
            continue;
        }
        out.push(with_host(|h| {
            h.new_str(utf16::Units::of(s).slice(p, m_start))
        }));
        if out.len() >= lim {
            return Ok(with_host(|h| h.new_array(out)));
        }
        for i in 1..caps.len() {
            out.push(match caps.get(i) {
                Some(g) => with_host(|h| h.new_str(g.as_str().to_string())),
                None => Value::Undef,
            });
            if out.len() >= lim {
                return Ok(with_host(|h| h.new_array(out)));
            }
        }
        p = e;
        q = p;
    }
    out.push(with_host(|h| h.new_str(utf16::Units::of(s).slice(p, size))));
    out.truncate(lim);
    Ok(with_host(|h| h.new_array(out)))
}

/// `str.replace(re, repl)` / `str.replaceAll(re, repl)`. `repl` is either a string
/// (with `$1`/`$&`/`` $` ``/`$'`/`$<name>`/`$$` patterns) or a function replacer.
pub fn str_replace_regex(
    s: &str,
    re_val: &Value,
    repl: &Value,
    all: bool,
) -> Result<Value, String> {
    let Some((re, global, _, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_str(s.to_string())));
    };
    let replace_all = all || global;
    let is_fn = with_host(|h| host::is_callable(h, repl));

    let mut out = String::new();
    let mut last = 0usize;
    let mut count = 0;
    for caps in re.captures_iter(s).flatten() {
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
            call_args.push(Value::Float(index_of_byte(s, m.start()).get() as f64));
            call_args.push(with_host(|h| h.new_str(s.to_string())));
            // 22.1.3.19: when the pattern has named groups the callback takes a
            // final `groups` argument. Omitting it left `arguments.length` at 4
            // where node reports 5, and destructuring the groups out of the last
            // parameter saw the subject string.
            if let Some(groups) = named_groups_object(&re, &caps) {
                call_args.push(groups);
            }
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
    // 22.2.6.11 step 8: a GLOBAL regexp has its `lastIndex` set to 0 by the
    // replace, so the next use starts from the beginning. It was left wherever
    // the caller had put it, which made a shared `/…/g` skip the front of the
    // string on its next `test`/`exec`.
    if global {
        set_last_index(re_val, U16Index::ZERO);
    }
    Ok(with_host(|h| h.new_str(out)))
}

/// Expand a replacement template's `$` patterns against a match.
/// The `groups` object for a match — `OrdinaryObjectCreate(null)` carrying each
/// named capture (22.2.7.2 step 30), or `None` when the pattern names none.
fn named_groups_object(re: &Regex, caps: &Captures) -> Option<Value> {
    let names: Vec<&str> = re.capture_names().flatten().collect();
    if names.is_empty() {
        return None;
    }
    let mut g: IndexMap<String, Value> = IndexMap::new();
    for name in names {
        let v = match caps.name(name) {
            Some(m) => with_host(|h| h.new_str(m.as_str().to_string())),
            None => Value::Undef,
        };
        g.insert(name.to_string(), v);
    }
    Some(with_host(|h| {
        let obj = h.new_object(g);
        let null = h.null();
        h.set_proto(&obj, null);
        obj
    }))
}

fn expand_replacement(templ: &str, caps: &Captures, s: &str) -> String {
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
