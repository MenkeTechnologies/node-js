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
//!   * `\N` / `\k<name>` → a conditional, so a reference to an unset group
//!     matches empty as in JS; the Annex B legacy escapes (`\0`, octal, `\cX`,
//!     identity `\8`/`\k`) become fixed code points; non-JS group syntax
//!     (`(?i)`, `(?P<n>`, `(?>`) is rejected with node's reason.
//!
//! Everything else — including `(?<name>...)`, `(?=)`/`(?!)`, `(?<=)`/`(?<!)`
//! and the `(?ims-ims:...)` modifier groups — passes through verbatim.
//!
//! Flags: `i`/`m`/`s` map onto inline flags; `g`/`y` drive iteration and
//! `lastIndex` here (fancy-regex has no global flag); `u`/`d` are accepted.

use crate::host::{self, with_host, JsObj, RegExpObj};
use crate::utf16::{self, U16Index};
use fancy_regex::{Captures, Match, Regex};
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

/// The early errors of a regex LITERAL (13.2.7.3): its flags and its pattern
/// grammar, in the wording V8 gives a literal. The flag message differs from
/// the constructor's, which names the argument.
pub fn check_literal(pattern: &str, flags: &str) -> Result<(), String> {
    let mut seen = String::new();
    for c in flags.chars() {
        if !"dgimsuvy".contains(c) || seen.contains(c) {
            return Err("SyntaxError: Invalid regular expression flags".to_string());
        }
        seen.push(c);
    }
    if flags.contains('u') && flags.contains('v') {
        return Err("SyntaxError: Invalid regular expression flags".to_string());
    }
    let sets = flags.contains('v');
    crate::regexp_syntax::validate(pattern, flags.contains('u') || sets, sets).map_err(|reason| {
        format!("SyntaxError: Invalid regular expression: /{pattern}/{flags}: {reason}")
    })
}

pub fn build_regexp(pattern: &str, flags: &str) -> Result<Value, String> {
    // Validate flags (Node throws on an unknown/repeated flag).
    let mut seen = String::new();
    for c in flags.chars() {
        // `u` and `v` are mutually exclusive (22.2.3.1 step 5).
        if !"dgimsuvy".contains(c)
            || seen.contains(c)
            || (c == 'v' && seen.contains('u'))
            || (c == 'u' && seen.contains('v'))
        {
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
    // `v` (unicodeSets) is `u` plus nested classes and the `--` / `&&` set
    // operators, which the regex layer spells the same way.
    let sets = flags.contains('v');
    let unicode = flags.contains('u') || sets;

    let invalid = |reason: &str| {
        format!("SyntaxError: Invalid regular expression: /{pattern}/{flags}: {reason}")
    };
    crate::regexp_syntax::validate(pattern, unicode, sets).map_err(|r| invalid(&r))?;
    let (rust_pat, capture_map) =
        translate(pattern, unicode, sets, dot_all, multiline).map_err(|r| invalid(&r))?;
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
        invalid(&e.lines().collect::<Vec<_>>().join(" "))
    })?;

    // A `RegExpObj` is unavoidably fresh per evaluation (`lastIndex` is
    // per-object mutable state), but the engine inside it is not.
    let obj = RegExpObj {
        re: Rc::new(Engine {
            re,
            map: Rc::new(capture_map),
        }),
        source: escape_regexp_pattern(pattern),
        flags: flags.to_string(),
        global,
        ignore_case,
        multiline,
        dot_all,
        sticky,
        unicode: flags.contains('u'),
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

/// The capturing groups of a JS pattern, in source order: how many there are,
/// and the index each named one got. A group is capturing when its `(` is not
/// escaped, not inside a class, and is either bare or `(?<name>`
/// (`(?<=`/`(?<!` are lookbehinds).
///
/// `translate` needs both BEFORE it reaches any escape: a decimal escape `\N` is
/// a backreference only when `N` does not exceed the pattern's TOTAL group count
/// (22.2.1.1 — a forward reference such as `/\1(a)/` still counts), and `\k` is
/// a named reference only when the pattern has a named group at all.
fn scan_groups(chars: &[char]) -> (usize, Vec<(String, usize)>) {
    let mut count = 0usize;
    let mut names = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => {
                if chars.get(i + 1) != Some(&'?') {
                    count += 1;
                } else if chars.get(i + 2) == Some(&'<')
                    && !matches!(chars.get(i + 3), Some('=') | Some('!'))
                {
                    count += 1;
                    let name: String = chars[i + 3..].iter().take_while(|c| **c != '>').collect();
                    names.push((name, count));
                }
            }
            _ => {}
        }
        i += 1;
    }
    (count, names)
}

/// What a JS capture group number means to the engine, and which captures are
/// STALE.
///
/// The engine keeps a capture from an earlier iteration of a quantified group
/// when the last iteration did not set it, where 22.2.2.5.1 RepeatMatcher step
/// 4 clears every capture inside the group at the start of each iteration:
/// `/(z)((a+)?(b+)?(c))*/.exec('zaacbbbcac')` is `bbb` in the engine and
/// `undefined` in JS. The engine cannot be told to reset, so the match is
/// corrected afterwards: a capture inside a repeating group is valid only when
/// it lies within the span the group's LAST iteration matched. A quantified
/// group that is not itself capturing is made capturing in the translated
/// pattern for exactly that reason, which shifts the engine's numbering —
/// `fancy_of_js` maps JS numbers to it.
///
/// The same spans carry the other half of RepeatMatcher: an iteration that
/// matches the empty string is rejected while the minimum is already met
/// (step 2.b), so a group under a quantifier whose minimum is 0 never holds an
/// empty capture — `/(a*)?/.exec('b')[1]` is `undefined`, where the engine
/// reports `''`.
///
/// What this cannot see: an empty capture left at the start of the last
/// iteration, and a backreference read inside the loop. A capture made inside
/// a lookaround is judged by where it STARTS (lookahead) or ENDS (lookbehind),
/// since the rest of its span may lie outside the iteration.
pub struct CaptureMap {
    /// Engine group number for each JS group number; index 0 is the whole match.
    fancy_of_js: Vec<usize>,
    /// Per ENGINE group number: the quantified groups enclosing it (engine
    /// numbers), outermost first, with how its span must sit inside theirs.
    guards: Vec<Vec<(usize, Within)>>,
    /// Per ENGINE group number: whether an empty capture is impossible.
    never_empty: Vec<bool>,
    /// Named groups with their JS group numbers.
    names: Vec<(String, usize)>,
}

/// How a capture's span is judged against the span of an enclosing group's
/// iteration.
#[derive(Clone, Copy)]
enum Within {
    /// The whole span lies inside.
    Whole,
    /// The span starts inside (a capture within a lookahead).
    Start,
    /// The span ends inside (a capture within a lookbehind).
    End,
}

impl CaptureMap {
    /// The identity numbering of a pattern with `count` capturing groups.
    fn identity(count: usize, names: Vec<(String, usize)>) -> CaptureMap {
        CaptureMap {
            fancy_of_js: (0..=count).collect(),
            guards: vec![Vec::new(); count + 1],
            never_empty: vec![false; count + 1],
            names,
        }
    }
}

/// A group's role while the pattern is scanned for repetition.
#[derive(Clone, Copy, PartialEq)]
enum GroupKind {
    Capture,
    /// `(?:…)`, the only kind that is rewritten into a capture.
    Plain,
    LookAhead,
    LookBehind,
    /// Anything else behind `(?` (modifier groups).
    Other,
}

/// The pre-pass result for one pattern: which `(?:` groups `translate` must
/// emit as captures, in `(` order, and the numbering that results.
struct GroupPlan {
    hidden: Vec<bool>,
    map: CaptureMap,
}

/// Whether a quantifier at `chars[at..]` can run its operand more than once:
/// `*`, `+`, `{n,}` and `{n,m}` with `m > 1`, `{n}` with `n > 1`.
fn quantifier_repeats(chars: &[char], at: usize) -> bool {
    match chars.get(at) {
        Some('*' | '+') => true,
        Some('{') => {
            let body: String = chars[at + 1..].iter().take_while(|c| **c != '}').collect();
            if chars.get(at + 1 + body.chars().count()) != Some(&'}') {
                return false;
            }
            match body.split_once(',') {
                None => body.parse::<u64>().is_ok_and(|n| n > 1),
                Some((lo, "")) => lo.parse::<u64>().is_ok(),
                Some((lo, hi)) => {
                    lo.parse::<u64>().is_ok() && hi.parse::<u64>().is_ok_and(|n| n > 1)
                }
            }
        }
        _ => false,
    }
}

/// Scan a pattern's group structure (the same notion of "a group" as
/// [`scan_groups`]) and decide the numbering described on [`CaptureMap`].
fn plan_groups(chars: &[char], names: Vec<(String, usize)>) -> GroupPlan {
    let mut kind: Vec<GroupKind> = Vec::new();
    let mut parent: Vec<Option<usize>> = Vec::new();
    let mut repeats: Vec<bool> = Vec::new();
    let mut min_zero: Vec<bool> = Vec::new();
    let mut has_capture: Vec<bool> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => {
                let k = if chars.get(i + 1) != Some(&'?') {
                    GroupKind::Capture
                } else {
                    match (chars.get(i + 2), chars.get(i + 3)) {
                        (Some(':'), _) => GroupKind::Plain,
                        (Some('=' | '!'), _) => GroupKind::LookAhead,
                        (Some('<'), Some('=' | '!')) => GroupKind::LookBehind,
                        (Some('<'), _) => GroupKind::Capture,
                        _ => GroupKind::Other,
                    }
                };
                parent.push(open.last().copied());
                open.push(kind.len());
                kind.push(k);
                repeats.push(false);
                min_zero.push(false);
                has_capture.push(false);
            }
            ')' if !in_class => {
                if let Some(id) = open.pop() {
                    repeats[id] = quantifier_repeats(chars, i + 1);
                    min_zero[id] = quantifier_at(chars, i + 1).is_some_and(|(_, min)| min == 0);
                    if kind[id] == GroupKind::Capture || has_capture[id] {
                        if let Some(p) = parent[id] {
                            has_capture[p] = true;
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    let hidden: Vec<bool> = (0..kind.len())
        .map(|g| kind[g] == GroupKind::Plain && (repeats[g] || min_zero[g]) && has_capture[g])
        .collect();
    // Engine numbers follow `(` order over capturing and rewritten groups.
    let mut fancy: Vec<usize> = vec![0; kind.len()];
    let mut fancy_of_js = vec![0usize];
    let mut next = 0;
    for g in 0..kind.len() {
        if kind[g] == GroupKind::Capture || hidden[g] {
            next += 1;
            fancy[g] = next;
            if kind[g] == GroupKind::Capture {
                fancy_of_js.push(next);
            }
        }
    }
    let mut guards: Vec<Vec<(usize, Within)>> = vec![Vec::new(); next + 1];
    let mut never_empty = vec![false; next + 1];
    for g in 0..kind.len() {
        if fancy[g] == 0 {
            continue;
        }
        never_empty[fancy[g]] = min_zero[g];
        let mut chain = Vec::new();
        let mut within = Within::Whole;
        let mut p = parent[g];
        while let Some(a) = p {
            // A capture in a lookaround is judged at one end only, and a
            // lookahead inside a lookbehind (or the reverse) at neither end.
            within = match (kind[a], within) {
                (GroupKind::LookAhead, Within::Whole | Within::Start) => Within::Start,
                (GroupKind::LookBehind, Within::Whole | Within::End) => Within::End,
                (GroupKind::LookAhead | GroupKind::LookBehind, _) => break,
                _ => within,
            };
            if fancy[a] != 0 && (repeats[a] || min_zero[a]) {
                chain.push((fancy[a], within));
            }
            p = parent[a];
        }
        chain.reverse();
        guards[fancy[g]] = chain;
    }
    GroupPlan {
        hidden,
        map: CaptureMap {
            fancy_of_js,
            guards,
            never_empty,
            names,
        },
    }
}

/// A compiled pattern together with its [`CaptureMap`]. Derefs to the engine
/// for everything that does not look at captures.
pub struct Engine {
    re: Rc<Regex>,
    map: Rc<CaptureMap>,
}

impl std::ops::Deref for Engine {
    type Target = Regex;
    fn deref(&self) -> &Regex {
        &self.re
    }
}

impl Engine {
    pub fn captures_from_pos<'t>(
        &self,
        s: &'t str,
        pos: usize,
    ) -> Result<Option<Caps<'t>>, fancy_regex::Error> {
        Ok(self.re.captures_from_pos(s, pos)?.map(|inner| Caps {
            inner,
            map: self.map.clone(),
        }))
    }

    pub fn captures<'t>(&self, s: &'t str) -> Result<Option<Caps<'t>>, fancy_regex::Error> {
        Ok(self.re.captures(s)?.map(|inner| Caps {
            inner,
            map: self.map.clone(),
        }))
    }
}

/// The captures of one match, numbered and filtered as JS sees them.
pub struct Caps<'t> {
    inner: Captures<'t>,
    map: Rc<CaptureMap>,
}

impl<'t> Caps<'t> {
    /// Number of JS groups, including the whole match.
    pub fn len(&self) -> usize {
        self.map.fancy_of_js.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Capture `i` as JS numbers it, or `None` if it did not participate in
    /// the last iteration of every repeating group around it.
    pub fn get(&self, i: usize) -> Option<Match<'t>> {
        let f = *self.map.fancy_of_js.get(i)?;
        self.valid(f).then(|| self.inner.get(f)).flatten()
    }

    pub fn name(&self, name: &str) -> Option<Match<'t>> {
        let (_, i) = self.map.names.iter().find(|(n, _)| n == name)?;
        self.get(*i)
    }

    fn valid(&self, f: usize) -> bool {
        let Some(m) = self.inner.get(f) else {
            return false;
        };
        if self.map.never_empty[f] && m.start() == m.end() {
            return false;
        }
        self.map.guards[f].iter().all(|&(q, within)| {
            self.inner.get(q).is_some_and(|g| {
                let inside = |at: usize| g.start() <= at && at <= g.end();
                match within {
                    Within::Whole => inside(m.start()) && inside(m.end()),
                    Within::Start => inside(m.start()),
                    Within::End => inside(m.end()),
                }
            }) && self.valid(q)
        })
    }
}

/// A code point as the fixed `\x{..}` spelling the regex layer accepts both in
/// and out of a class.
fn hex_escape(cp: u32) -> String {
    format!("\\x{{{cp:X}}}")
}

/// Annex B.1.2 LegacyOctalEscapeSequence starting at `chars[i]` (an octal
/// digit): the longest of `[0-3][0-7][0-7]`, `[0-7][0-7]`, `[0-7]`, so the value
/// never exceeds 0o377. Returns the code point and how many digits it took.
fn legacy_octal(chars: &[char], i: usize) -> (u32, usize) {
    let oct = |k: usize| chars.get(k).and_then(|c| c.to_digit(8));
    let first = oct(i).unwrap_or(0);
    let max = if first <= 3 { 3 } else { 2 };
    let mut value = first;
    let mut len = 1;
    while len < max {
        match oct(i + len) {
            Some(d) => {
                value = value * 8 + d;
                len += 1;
            }
            None => break,
        }
    }
    (value, len)
}

/// Validate the group opener at `chars[i] == '('` when it is followed by `?`,
/// returning the reason node's parser gives for a form JS does not have.
///
/// JS accepts exactly `(?:`, `(?=`, `(?!`, `(?<=`, `(?<!`, `(?<name>`, and the
/// ES2025 modifier groups `(?ims-ims:` — never a bare inline flag (`(?i)`), and
/// none of Perl/PCRE's `(?x)`, `(?P<n>`, `(?#…)`, `(?>…)`, all of which the
/// regex layer would otherwise accept and silently give a meaning.
fn check_group(chars: &[char], i: usize) -> Result<(), &'static str> {
    match chars.get(i + 2) {
        Some(':') | Some('=') | Some('!') | Some('<') => return Ok(()),
        _ => {}
    }
    let mut seen = String::new();
    let mut k = i + 2;
    let mut saw_dash = false;
    while let Some(&c) = chars.get(k) {
        match c {
            'i' | 'm' | 's' => {
                if seen.contains(c) {
                    return Err("Repeated flag in flag group");
                }
                seen.push(c);
            }
            '-' if !saw_dash => saw_dash = true,
            ':' if seen.is_empty() => return Err("Invalid flag group"),
            ':' => return Ok(()),
            _ => return Err("Invalid group"),
        }
        k += 1;
    }
    Err("Invalid group")
}

/// `\b` and `\B` over the ASCII word set JS defines (22.2.2.9 IsWordChar). The
/// regex layer's own `\b` is Unicode-aware and its `(?-u:\b)` is refused by
/// fancy-regex, so the assertion is spelled out with lookaround.
const WORD_BOUNDARY: &str =
    "(?:(?<=[0-9A-Za-z_])(?![0-9A-Za-z_])|(?<![0-9A-Za-z_])(?=[0-9A-Za-z_]))";
const NOT_WORD_BOUNDARY: &str =
    "(?:(?<=[0-9A-Za-z_])(?=[0-9A-Za-z_])|(?<![0-9A-Za-z_])(?![0-9A-Za-z_]))";

/// A quantifier at `chars[at..]` — `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, each
/// with an optional lazy `?` — as (index past it, its minimum count).
fn quantifier_at(chars: &[char], at: usize) -> Option<(usize, u64)> {
    let (mut end, min) = match chars.get(at)? {
        '*' | '?' => (at + 1, 0),
        '+' => (at + 1, 1),
        '{' => {
            let mut j = at + 1;
            let lo_start = j;
            while chars.get(j).is_some_and(char::is_ascii_digit) {
                j += 1;
            }
            if j == lo_start {
                return None;
            }
            let min: u64 = chars[lo_start..j]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(u64::MAX);
            if chars.get(j) == Some(&',') {
                j += 1;
                while chars.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
            }
            if chars.get(j) != Some(&'}') {
                return None;
            }
            (j + 1, min)
        }
        _ => return None,
    };
    if chars.get(end) == Some(&'?') {
        end += 1;
    }
    Some((end, min))
}

/// Whether four hex digits sit at `chars[at..]` — the `\uXXXX` form.
fn four_hex(chars: &[char], at: usize) -> bool {
    chars
        .get(at..at + 4)
        .is_some_and(|h| h.iter().all(|c| c.is_ascii_hexdigit()))
}

/// The JS meaning of `\d \D \w \W \s \S` (22.2.2.9) in the regex layer's
/// syntax. Inside a class the positive forms are bare members and the negated
/// ones nested classes, which the regex layer unions.
fn class_escape(e: char, in_class: bool) -> String {
    /// WhiteSpace and LineTerminator (12.2, 12.3) — which is NOT Unicode's
    /// `White_Space`: it has U+FEFF and lacks U+0085.
    const WS: &str = r"\t\n\x{B}\x{C}\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
    const WORD: &str = "0-9A-Za-z_";
    let members = match e.to_ascii_lowercase() {
        'd' => "0-9",
        'w' => WORD,
        _ => WS,
    };
    match (e.is_ascii_lowercase(), in_class) {
        (true, true) => members.to_string(),
        (true, false) => format!("[{members}]"),
        (false, _) => format!("[^{members}]"),
    }
}

/// Translate a JS regex source into fancy-regex syntax, or the reason node's
/// parser would reject it (the caller adds the `Invalid regular expression:
/// /…/flags: ` frame).
///
/// Rewrites, each because the regex layer reads the same spelling differently:
///   * `\uXXXX` / `\u{…}` → `\x{…}`, surrogates remapped (see `remap_surrogate`).
///   * `\/` → `/`; a bare `[` inside a class → `\[`.
///   * `\N` that names an existing group → `(?(N)\N|)`. JS matches a reference
///     to a group that has not participated as the EMPTY string (22.2.2.7.2
///     BackreferenceMatcher step 7), so `/\1(a)/` and `/(a)?b\1/` both match;
///     fancy-regex fails such a reference instead, and its conditional is what
///     restores "empty when unset". `\k<name>` gets the same treatment by index.
///   * Outside unicode mode, the Annex B.1.2 legacy forms: `\N` past the group
///     count is an octal escape (`\052` is `*`), or the digit itself for 8/9;
///     `\0` is NUL; in a class every `\N` is octal; `\cX` is the control
///     character X % 32 (plus `\c<digit>`/`\c_` in a class), and a `\c` that
///     forms none of those is a literal backslash followed by `c`; `\k` with no
///     named group in the pattern is the letter `k`.
///   * In unicode mode those legacy forms are the SyntaxErrors node raises.
fn translate(
    pat: &str,
    unicode: bool,
    sets: bool,
    dot_all: bool,
    multiline: bool,
) -> Result<(String, CaptureMap), String> {
    match translate_with(pat, unicode, sets, dot_all, multiline, true)? {
        (out, map, true) => Ok((out, map)),
        // The pre-pass and the walk disagreed about the number of groups (a
        // `v`-mode class holding a `(`), which would misnumber every capture
        // after it: translate again with the engine's own numbering, which is
        // merely un-filtered.
        _ => translate_with(pat, unicode, sets, dot_all, multiline, false).map(|(o, m, _)| (o, m)),
    }
}

/// [`translate`] with `allow_hidden` choosing whether repeating `(?:…)` groups
/// are rewritten into captures. The flag in the result says the group count the
/// pre-pass planned for matched the one the walk saw.
fn translate_with(
    pat: &str,
    unicode: bool,
    sets: bool,
    dot_all: bool,
    multiline: bool,
    allow_hidden: bool,
) -> Result<(String, CaptureMap, bool), String> {
    let chars: Vec<char> = pat.chars().collect();
    let (group_count, group_names) = scan_groups(&chars);
    let mut plan = plan_groups(&chars, group_names.clone());
    if !allow_hidden {
        plan = GroupPlan {
            hidden: vec![false; plan.hidden.len()],
            map: CaptureMap::identity(group_count, group_names.clone()),
        };
    }
    // How many source `(` have been seen, to index `plan.hidden` in step with
    // the pre-pass.
    let mut paren_seen = 0usize;
    let mut out = String::new();
    let mut i = 0;
    // Track whether we're inside a `[...]` class. `class_pos` is how many chars
    // into the current class we are, so we can spot the `]` that would close an
    // empty class (`[]` / `[^]`) vs. a literal leading `]`.
    let mut in_class = false;
    let mut class_pos = 0usize;
    // Under `v` classes nest; this counts the open ones.
    let mut class_depth = 0usize;
    // Whether the previous class member was a set escape (`\d`): a `-` after one
    // is a literal, which the regex layer would otherwise read as a range.
    let mut prev_set = false;
    // One entry per open group: where its `(` landed in `out` when it is a
    // lookahead (which Annex B lets take a quantifier — the regex layer does not,
    // so it is wrapped in `(?:…)` once its close is seen).
    let mut group_open: Vec<Option<usize>> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        let after_set = std::mem::take(&mut prev_set);
        // Character-class bookkeeping. A `\` escape is handled below and never
        // toggles class state (it consumes its own two chars).
        if c != '\\' {
            if (!in_class || sets) && c == '[' {
                // A `]` right after the opener (and its `^`) CLOSES the class in
                // JS: `[]` matches nothing and `[^]` matches any code unit.
                let negated = chars.get(i + 1) == Some(&'^');
                let close_at = i + 1 + usize::from(negated);
                if chars.get(close_at) == Some(&']') {
                    out.push_str(if negated { r"[\s\S]" } else { r"[^\s\S]" });
                    if in_class {
                        class_pos += 1;
                    }
                    i = close_at + 1;
                    continue;
                }
                in_class = true;
                class_depth += 1;
                class_pos = 0;
                out.push('[');
                i += 1;
                // A leading `^` is the negation, not the first member.
                if negated {
                    out.push('^');
                    i += 1;
                }
                continue;
            }
            if in_class {
                // The first char of a class, if `]`, is a literal `]` in JS; a
                // later bare `[` must be escaped for the regex layer.
                if c == ']' && (class_pos > 0 || sets) {
                    class_depth = class_depth.saturating_sub(1);
                    in_class = class_depth > 0;
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
            } else if c == '(' && chars.get(i + 1) == Some(&'?') {
                check_group(&chars, i).map_err(str::to_string)?;
            }
        }
        match c {
            '\\' => {
                class_pos += 1;
                match chars.get(i + 1).copied() {
                    // `\uXXXX` / `\u{...}` → `\x{...}` (surrogates remapped). Outside the
                    // `u` flag only the four-digit form is an escape: `\u{61}` is `u`
                    // repeated 61 times, and a malformed `\u` is the letter itself.
                    Some('u') if !unicode && !four_hex(&chars, i + 2) => {
                        out.push('u');
                        i += 2;
                        continue;
                    }
                    Some('u') if !unicode && chars.get(i + 2) == Some(&'{') => {
                        out.push('u');
                        i += 2;
                        continue;
                    }
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
                            Ok(cp) => out.push_str(&hex_escape(remap_surrogate(cp))),
                            // Not valid hex — emit the code point literally so the
                            // engine surfaces its own error rather than us guessing.
                            Err(_) => out.push_str(&format!("\\x{{{cp_hex}}}")),
                        }
                        continue;
                    }
                    // `\xHH`; a malformed one outside `u` is the letter `x` (B.1.2).
                    Some('x') => {
                        let hex: String = chars
                            .get(i + 2..i + 4)
                            .map(|h| h.iter().collect())
                            .unwrap_or_default();
                        match u32::from_str_radix(&hex, 16) {
                            Ok(v)
                                if hex.len() == 2 && hex.chars().all(|c| c.is_ascii_hexdigit()) =>
                            {
                                out.push_str(&hex_escape(v));
                                i += 4;
                            }
                            _ => {
                                out.push('x');
                                i += 2;
                            }
                        }
                        continue;
                    }
                    // `\/` in a JS literal → a plain slash (regex rejects `\/`).
                    Some('/') => {
                        out.push('/');
                        i += 2;
                        continue;
                    }
                    Some('c') => {
                        let control = match chars.get(i + 2) {
                            Some(x) if x.is_ascii_alphabetic() => Some(*x),
                            Some(x)
                                if in_class && !unicode && (x.is_ascii_digit() || *x == '_') =>
                            {
                                Some(*x)
                            }
                            _ => None,
                        };
                        match control {
                            Some(x) => {
                                out.push_str(&hex_escape(x as u32 % 32));
                                i += 3;
                            }
                            None if unicode => return Err("Invalid Unicode escape".into()),
                            // Annex B: `\` stands for itself; `c` is read next.
                            None => {
                                out.push_str("\\\\");
                                i += 1;
                            }
                        }
                        continue;
                    }
                    Some(d) if d.is_ascii_digit() => {
                        let lone_zero =
                            d == '0' && !chars.get(i + 2).is_some_and(|n| n.is_ascii_digit());
                        if lone_zero {
                            out.push_str(&hex_escape(0));
                            i += 2;
                            continue;
                        }
                        if !in_class && d != '0' {
                            let digits: String = chars[i + 1..]
                                .iter()
                                .take_while(|c| c.is_ascii_digit())
                                .collect();
                            let n = digits.parse::<usize>().unwrap_or(usize::MAX);
                            if n <= group_count {
                                let n = plan.map.fancy_of_js[n];
                                out.push_str(&format!("(?({n})\\{n}|)"));
                                i += 1 + digits.len();
                                continue;
                            }
                        }
                        if unicode {
                            let reason = if in_class || d == '0' {
                                "Invalid decimal escape"
                            } else {
                                "Invalid escape"
                            };
                            return Err(reason.into());
                        }
                        if d == '8' || d == '9' {
                            out.push(d);
                            i += 2;
                        } else {
                            let (cp, len) = legacy_octal(&chars, i + 1);
                            out.push_str(&hex_escape(cp));
                            i += 1 + len;
                        }
                        continue;
                    }
                    Some('k') if in_class || (group_names.is_empty() && !unicode) => {
                        if unicode {
                            return Err("Invalid class escape".into());
                        }
                        out.push('k');
                        i += 2;
                        continue;
                    }
                    Some('k') => {
                        if chars.get(i + 2) != Some(&'<') {
                            return Err("Invalid named reference".into());
                        }
                        let Some(close) = chars[i + 3..].iter().position(|c| *c == '>') else {
                            return Err("Invalid capture group name".into());
                        };
                        let name: String = chars[i + 3..i + 3 + close].iter().collect();
                        let Some((_, index)) = group_names.iter().find(|(n, _)| *n == name) else {
                            return Err("Invalid named capture referenced".into());
                        };
                        let index = plan.map.fancy_of_js[*index];
                        out.push_str(&format!("(?({index})\\{index}|)"));
                        i += 4 + close;
                        continue;
                    }
                    // `\p{…}` is a property escape only under `u`/`v`; without them it
                    // is the letter itself, followed by whatever `{…}` is.
                    Some(pc @ ('p' | 'P')) if !unicode => {
                        out.push(pc);
                        i += 2;
                        continue;
                    }
                    // The class escapes are ASCII-only in JS (and `\s` is the spec's
                    // WhiteSpace + LineTerminator list), where the regex layer's own
                    // `\d \w \s \b` are Unicode-aware.
                    Some(e @ ('d' | 'D' | 'w' | 'W' | 's' | 'S')) => {
                        out.push_str(&class_escape(e, in_class));
                        prev_set = in_class;
                        i += 2;
                        continue;
                    }
                    Some('b') if in_class => {
                        out.push_str(&hex_escape(8));
                        i += 2;
                        continue;
                    }
                    Some(b @ ('b' | 'B')) => {
                        out.push_str(if b == 'b' {
                            WORD_BOUNDARY
                        } else {
                            NOT_WORD_BOUNDARY
                        });
                        i += 2;
                        continue;
                    }
                    // Everything else (`\n \. \\` …) passes through.
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
            '(' if !in_class => {
                let source_paren = paren_seen;
                paren_seen += 1;
                // A repeating `(?:…)` that holds captures becomes a capture so
                // its last iteration's span is known (see `CaptureMap`).
                if plan.hidden.get(source_paren) == Some(&true) {
                    group_open.push(None);
                    out.push('(');
                    i += 3;
                    continue;
                }
                let lookahead =
                    chars.get(i + 1) == Some(&'?') && matches!(chars.get(i + 2), Some('=' | '!'));
                group_open.push(lookahead.then_some(out.len()));
                out.push('(');
                i += 1;
            }
            ')' if !in_class => {
                out.push(')');
                i += 1;
                // A quantified lookahead (Annex B) cannot be handed to the regex
                // layer, which refuses to repeat a zero-width assertion. It does
                // not need to: an iteration that matches the empty string is
                // rejected once the minimum is met (22.2.2.3.1 RepeatMatcher
                // step 2.b), so the quantifier can only ever run the assertion
                // ONCE when its minimum is 1 or more, and never when it is 0.
                if let Some(Some(at)) = group_open.pop() {
                    if let Some((next, min)) = quantifier_at(&chars, i).filter(|_| !unicode) {
                        if min == 0 {
                            // Kept as a group that can never match, so the
                            // numbering of any capture inside is unchanged.
                            out.insert_str(at, "(?:(?!)");
                            out.push_str(")?");
                        }
                        i = next;
                    }
                }
            }
            // `.` excludes all four LineTerminators, not just `\n`.
            '.' if !in_class && !dot_all => {
                out.push_str(r"[^\n\r\x{2028}\x{2029}]");
                i += 1;
            }
            // Under `m`, `^`/`$` also sit beside `\r`, LS and PS; the engine's own
            // multi-line anchors know only `\n`.
            '^' if !in_class && multiline => {
                out.push_str(r"(?:^|(?<=[\r\x{2028}\x{2029}]))");
                i += 1;
            }
            '$' if !in_class && multiline => {
                out.push_str(r"(?:$|(?=[\r\x{2028}\x{2029}]))");
                i += 1;
            }
            '-' if in_class
                && (after_set
                    || (chars.get(i + 1) == Some(&'\\')
                        && matches!(
                            chars.get(i + 2),
                            Some('d' | 'D' | 'w' | 'W' | 's' | 'S')
                        ))) =>
            {
                out.push_str("\\-");
                class_pos += 1;
                i += 1;
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
    Ok((out, plan.map, paren_seen == plan.hidden.len()))
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
        // node says `true`/`false`.
        "hasIndices" => Value::Bool(r.flags.contains('d')),
        "unicodeSets" => Value::Bool(r.flags.contains('v')),
        "lastIndex" => Value::Float(r.last_index.get() as f64),
        _ => return None,
    })
}

pub fn is_regexp_method(name: &str) -> bool {
    matches!(name, "test" | "exec" | "toString" | "compile")
        // The five symbol-keyed methods a RegExp exposes so the string methods
        // can delegate to it (22.2.6.x). They were absent, so
        // `/a/[Symbol.match]("x")` was not a function and a subclass could not
        // override the protocol by calling `super[Symbol.match]`.
        || matches!(
            name,
            "@@match" | "@@matchAll" | "@@search" | "@@split" | "@@replace"
        )
}

/// V8's wording for a `RegExp.prototype` method whose receiver is not a RegExp.
pub fn incompatible_receiver(method: &str, recv: &Value) -> String {
    host::type_error(&format!(
        "Method RegExp.prototype.{method} called on incompatible receiver {}",
        crate::builtins::no_side_effects_string_pub(recv)
    ))
}

/// Whether `f` is the built-in `RegExp.prototype.exec` — read off a RegExp
/// (a thunk bound to it) or off the prototype itself.
fn is_builtin_exec(f: &Value) -> bool {
    with_host(|h| match h.get(f) {
        Some(JsObj::BoundMethod { recv, name }) => {
            name == "exec" && matches!(h.get(recv), Some(JsObj::RegExp(_)))
        }
        Some(JsObj::Builtin(n)) => n == "@proto:RegExp:exec",
        _ => false,
    })
}

/// `RegExpExec(R, S)` (22.2.7.1). `R`'s `exec` is read with `[[Get]]`; one that
/// is not the built-in — an own override, a subclass method, or the method of a
/// plain object — is called and must return an object or `null`. Otherwise `R`
/// has to be a RegExp, and the built-in matcher runs.
pub fn regexp_exec_generic(r: &Value, s: &str) -> Result<Value, String> {
    match user_exec(r)? {
        Some(exec) => call_user_exec(r, &exec, s),
        None => regexp_exec(require_regexp(r, "exec")?, s),
    }
}

/// The `exec` `RegExpExec` must CALL: `Some` when `[[Get]](R, "exec")` is a
/// callable other than the built-in, `None` when the built-in matcher applies.
fn user_exec(r: &Value) -> Result<Option<Value>, String> {
    // The common case answers without materializing the method: a RegExp with
    // no own `exec`, linked straight to `RegExp.prototype`, whose `exec` no
    // script has replaced (a replacement lives in the namespace's own table).
    let untouched = with_host(|h| {
        matches!(h.get(r), Some(JsObj::RegExp(_)))
            && h.fn_prop(r, "exec").is_none()
            && h.proto_of(r).is_none()
            && h.builtin_static("RegExp.prototype", "exec").is_none()
    });
    if untouched {
        return Ok(None);
    }
    let exec = crate::builtins::get_property(r, "exec")?;
    Ok((!is_builtin_exec(&exec) && with_host(|h| host::is_callable(h, &exec))).then_some(exec))
}

/// Call a user `exec` and enforce `RegExpExec` step 2.b on its result.
fn call_user_exec(r: &Value, exec: &Value, s: &str) -> Result<Value, String> {
    let subject = with_host(|h| h.new_str(s));
    let result = host::invoke(exec, vec![subject], Some(r.clone()))?;
    let ok = with_host(|h| {
        h.is_null(&result) || (matches!(result, Value::Obj(_)) && !host::is_primitive(h, &result))
    });
    if !ok {
        return Err(host::type_error(
            "RegExp exec method returned something other than an Object or null",
        ));
    }
    Ok(result)
}

/// `r` itself when it is a RegExp, else V8's incompatible-receiver error for
/// `method` — the built-in matcher needs `[[RegExpMatcher]]`.
fn require_regexp<'a>(r: &'a Value, method: &str) -> Result<&'a Value, String> {
    if with_host(|h| matches!(h.get(r), Some(JsObj::RegExp(_)))) {
        Ok(r)
    } else {
        Err(incompatible_receiver(method, r))
    }
}

/// `RegExp.prototype.test` (22.2.6.16): `RegExpExec` and a non-null result.
/// Generic over any object receiver; with the built-in `exec` it runs the
/// matcher without building a match array.
pub fn regexp_test_generic(r: &Value, arg: &Value) -> Result<Value, String> {
    let s = match with_host(|h| h.as_str(arg)) {
        Some(s) => s,
        None => {
            let v = host::to_string_value(arg)?;
            with_host(|h| h.str_of(&v))
        }
    };
    Ok(Value::Bool(match user_exec(r)? {
        Some(exec) => {
            let result = call_user_exec(r, &exec, &s)?;
            !with_host(|h| h.is_null(&result))
        }
        None => regexp_test(require_regexp(r, "exec")?, &s),
    }))
}

/// `RegExp.prototype.toString` (22.2.6.17): `"/" + ToString(R.source) + "/" +
/// ToString(R.flags)`, both read with `[[Get]]`, for any object receiver.
pub fn regexp_to_string_generic(r: &Value) -> Result<Value, String> {
    let read = |k: &str| -> Result<String, String> {
        let v = crate::builtins::get_property(r, k)?;
        let s = host::to_string_value(&v)?;
        Ok(with_host(|h| h.str_of(&s)))
    };
    let source = read("source")?;
    let flags = read("flags")?;
    Ok(with_host(|h| h.new_str(format!("/{source}/{flags}"))))
}

/// Dispatch a `RegExp.prototype` method.
pub fn regexp_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "test" => regexp_test_generic(recv, &args.first().cloned().unwrap_or(Value::Undef)),
        "exec" => {
            let s = host::to_string_value(&args.first().cloned().unwrap_or(Value::Undef))?;
            let s = with_host(|h| h.str_of(&s));
            regexp_exec(recv, &s)
        }
        "toString" => Ok(with_host(|h| {
            let s = h.str_of(recv);
            h.new_str(s)
        })),
        // `compile` is a legacy no-op here (the pattern is already compiled).
        "compile" => Ok(recv.clone()),
        // The symbol-keyed forms ARE the string methods' implementations, so
        // each forwards to the same routine with the arguments swapped: the
        // subject is the argument here and the receiver there.
        "@@match" | "@@matchAll" | "@@search" | "@@split" | "@@replace" => {
            let s = with_host(|h| h.str_of(&args.first().cloned().unwrap_or(Value::Undef)));
            match name {
                "@@match" => str_match(&s, recv),
                "@@matchAll" => str_match_all(&s, recv),
                "@@search" => str_search(&s, recv),
                "@@split" => {
                    let limit = args.get(1).and_then(|v| {
                        let n = with_host(|h| h.to_number(v));
                        n.is_finite().then_some(n.max(0.0) as usize)
                    });
                    str_split_regex(&s, recv, limit)
                }
                _ => str_replace_regex(
                    &s,
                    recv,
                    &args.get(1).cloned().unwrap_or(Value::Undef),
                    false,
                ),
            }
        }
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// Snapshot the fields we need without holding the host borrow across a match.
fn regexp_snapshot(recv: &Value) -> Option<(Rc<Engine>, bool, bool, U16Index)> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::RegExp(r)) => Some((r.re.clone(), r.global, r.sticky, r.last_index)),
        _ => None,
    })
}

/// Whether the regexp carries the `d` flag, so its matches report `.indices`.
fn has_indices(recv: &Value) -> bool {
    with_host(|h| match h.get(recv) {
        Some(JsObj::RegExp(r)) => r.flags.contains('d'),
        _ => false,
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
    Ok(build_match_array(&re, &caps, s, has_indices(recv)))
}

/// Build the JS match-result array from a `Captures`, attaching `.index`,
/// `.input`, and (named-group) `.groups` — plus `.indices` when the regexp
/// carried the `d` flag (22.2.7.2 step 34, `MakeMatchIndicesIndexPairArray`).
fn build_match_array(re: &Engine, caps: &Caps, s: &str, indices: bool) -> Value {
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
    if indices {
        attach_indices(caps, s, &arr, &names);
    }
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

/// `MakeMatchIndicesIndexPairArray` (22.2.7.8): a `[start, end]` pair per
/// capture — `null` for one that did not participate — with a null-prototype
/// `.groups` mirroring the named groups. Offsets are UTF-16 indices, the same
/// units `.index` uses.
fn attach_indices(caps: &Caps, s: &str, arr: &Value, names: &[&str]) {
    let pair = |m: Option<fancy_regex::Match>| match m {
        Some(m) => {
            let (a, b) = (index_of_byte(s, m.start()), index_of_byte(s, m.end()));
            with_host(|h| {
                h.new_array(vec![
                    Value::Float(a.get() as f64),
                    Value::Float(b.get() as f64),
                ])
            })
        }
        None => Value::Undef,
    };
    let mut pairs: Vec<Value> = Vec::with_capacity(caps.len());
    for i in 0..caps.len() {
        pairs.push(pair(caps.get(i)));
    }
    let idx_arr = with_host(|h| h.new_array(pairs));
    if names.is_empty() {
        with_host(|h| h.set_fn_prop(&idx_arr, "groups", Value::Undef));
    } else {
        let mut g: IndexMap<String, Value> = IndexMap::new();
        for name in names {
            g.insert((*name).to_string(), pair(caps.name(name)));
        }
        with_host(|h| {
            let obj = h.new_object(g);
            let null = h.null();
            h.set_proto(&obj, null);
            h.set_fn_prop(&idx_arr, "groups", obj);
        });
    }
    with_host(|h| h.set_fn_prop(arr, "indices", idx_arr));
}

// ── String.prototype regex methods (called from builtins::string_method) ──────

/// Every match `RegExpBuiltinExec` produces when driven in a loop from byte
/// offset `start` (22.2.6.9 / 22.2.6.11 / 22.2.9.1): each next search begins
/// where the last match ended, an EMPTY match steps one character further, and
/// under `y` a match must begin exactly where the search does — so a sticky
/// scan ends at the first gap rather than skipping over it.
fn collect_matches<'s>(re: &Engine, s: &'s str, start: usize, sticky: bool) -> Vec<Caps<'s>> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos <= s.len() {
        let Some(caps) = re.captures_from_pos(s, pos).ok().flatten() else {
            break;
        };
        let m = caps.get(0).expect("group 0 always participates");
        if sticky && m.start() != pos {
            break;
        }
        pos = match m.end() == m.start() {
            true => match s[m.end()..].chars().next() {
                Some(c) => m.end() + c.len_utf8(),
                None => s.len() + 1,
            },
            false => m.end(),
        };
        out.push(caps);
    }
    out
}

/// `str.match(re)`: without `g`, same as `exec` (array or null); with `g`, an
/// array of every whole-match string (or null if none).
pub fn str_match(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, global, sticky, _)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.null()));
    };
    if !global {
        // A sticky match is an `exec`: it starts at, and advances, `lastIndex`.
        if sticky {
            return regexp_exec(re_val, s);
        }
        // Otherwise it ignores `lastIndex`, searches from the start and — it
        // being neither `g` nor `y` — leaves it exactly where it was.
        return regexp_exec_from_zero(&re, s, has_indices(re_val));
    }
    // 22.2.6.9 step 6.a: a global match sets `lastIndex` to 0 before it starts,
    // so it always collects from the beginning and leaves it there. It was
    // being left wherever the caller had put it.
    set_last_index(re_val, U16Index::ZERO);
    let matches: Vec<Value> = collect_matches(&re, s, 0, sticky)
        .iter()
        .map(|c| with_host(|h| h.new_str(c.get(0).map_or("", |m| m.as_str()).to_string())))
        .collect();
    if matches.is_empty() {
        Ok(with_host(|h| h.null()))
    } else {
        Ok(with_host(|h| h.new_array(matches)))
    }
}

/// Non-global exec searching from offset 0 (for `str.match` without `g`).
fn regexp_exec_from_zero(re: &Engine, s: &str, indices: bool) -> Result<Value, String> {
    match re.captures(s).ok().flatten() {
        Some(caps) => Ok(build_match_array(re, &caps, s, indices)),
        None => Ok(with_host(|h| h.null())),
    }
}

/// `str.matchAll(re)`: an iterator over every match array (requires the `g` flag
/// in Node, but we accept a non-global regex too and still iterate all matches).
pub fn str_match_all(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, global, sticky, last)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_array(Vec::new())));
    };
    let indices = has_indices(re_val);
    // 22.1.3.14 step 5.c: a non-global regexp is a TypeError, because
    // `matchAll` cannot produce every match without `g` — the same rule
    // `replaceAll` enforces. This used to return just the first match.
    if !global {
        return Err(host::type_error(
            "String.prototype.matchAll called with a non-global RegExp argument",
        ));
    }
    // 22.2.6.9 `@@matchAll` clones the regexp with the ORIGINAL's `lastIndex`,
    // so the scan begins there rather than at 0.
    let mut items = Vec::new();
    if last.get() <= utf16::len(s) {
        let start = byte_of_index(s, last);
        for caps in collect_matches(&re, s, start, sticky) {
            items.push(build_match_array(&re, &caps, s, indices));
        }
    }
    // Return a live iterator so `for-of`/spread/`Array.from` all work.
    Ok(with_host(|h| {
        h.alloc(JsObj::Iter {
            items,
            idx: 0,
            array: None,
            brand: Some("RegExp String Iterator"),
        })
    }))
}

/// `str.search(re)`: char index of the first match, or -1.
pub fn str_search(s: &str, re_val: &Value) -> Result<Value, String> {
    let Some((re, _, sticky, _)) = regexp_snapshot(re_val) else {
        return Ok(Value::Float(-1.0));
    };
    // 22.2.6.12 runs the exec with `lastIndex` 0, so a sticky regexp can only
    // match AT the start; `lastIndex` itself is restored afterwards.
    Ok(match re.find(s).ok().flatten() {
        Some(m) if !sticky || m.start() == 0 => {
            Value::Float(index_of_byte(s, m.start()).get() as f64)
        }
        _ => Value::Float(-1.0),
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
    let Some((re, global, sticky, last_index)) = regexp_snapshot(re_val) else {
        return Ok(with_host(|h| h.new_str(s.to_string())));
    };
    let replace_all = all || global;
    let has_named_groups = re.capture_names().flatten().next().is_some();
    let is_fn = with_host(|h| host::is_callable(h, repl));

    let mut out = String::new();
    let mut last = 0usize;
    let mut count = 0;
    // Which matches the replace sees. A global one collects them all from 0; a
    // sticky one that is not global makes ONE `exec` at `lastIndex` and moves
    // `lastIndex` to its end (or back to 0 on failure); a plain one takes the
    // first match from the start.
    let found: Vec<Caps> = if global {
        collect_matches(&re, s, 0, sticky)
    } else if sticky {
        let all_from_here = if last_index.get() <= utf16::len(s) {
            collect_matches(&re, s, byte_of_index(s, last_index), true)
        } else {
            Vec::new()
        };
        let first: Vec<Caps> = all_from_here.into_iter().take(1).collect();
        let next = first
            .first()
            .and_then(|c| c.get(0))
            .map_or(U16Index::ZERO, |m| index_of_byte(s, m.end()));
        set_last_index(re_val, next);
        first
    } else {
        collect_matches(&re, s, 0, false)
    };
    for caps in found {
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
            out.push_str(&expand_replacement(&repl_str, &caps, s, has_named_groups));
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
fn named_groups_object(re: &Engine, caps: &Caps) -> Option<Value> {
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

fn expand_replacement(templ: &str, caps: &Caps, s: &str, has_named_groups: bool) -> String {
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
                // `$<name>` is a named-group reference only when the pattern has
                // named groups (22.1.3.19.1 GetSubstitution): without any, `$<`
                // is literal text, and so is `$<` with no closing `>`.
                '<' if has_named_groups && chars[i + 2..].contains(&'>') => {
                    let close = i + 2 + chars[i + 2..].iter().position(|c| *c == '>').unwrap_or(0);
                    let name: String = chars[i + 2..close].iter().collect();
                    if let Some(m) = caps.name(&name) {
                        out.push_str(m.as_str());
                    }
                    i = close + 1;
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
