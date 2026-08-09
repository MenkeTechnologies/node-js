//! Structural guard on the NAME-keyed registries — the sibling hazard to
//! `opcode_ids.rs`, which guards only the hand-numbered `u16` builtin id space.
//!
//! Where a builtin op is reached by integer, everything else in this frontend is
//! reached by *string*: `require('fs')` resolves a namespace name, `fs.readFile`
//! resolves a method name inside it, `buf.slice` resolves an instance-method name
//! against a `@@native` tag, and `host::ensure_ctor_proto` turns those same name
//! lists into real `.prototype` objects by `IndexMap::insert`-ing one key per
//! name. Every one of those lookups is last-write-wins: insert `"slice"` twice
//! and the first entry is simply gone, with no build error and no runtime
//! complaint.
//!
//! rustc already covers one slice of this: a literally duplicated `match` arm
//! (`"a" => .., "a" => ..`) is an `unreachable_patterns` warning. What it cannot
//! see is a duplicate inside a `&[&str]` table, the same name appearing in two
//! tables that get concatenated into one prototype, a namespace that `resolve`
//! hands out but no table backs, or a prototype tag claimed by two different
//! prototype maps. Those are the cases below.
//!
//! Tables are read out of the source TEXT rather than out of the linked crate
//! wherever a duplicate would be invisible post-compile (a `Vec` built from a
//! table with a repeated entry still has the repeat, but a map built from it does
//! not — the evidence is destroyed by the very insert this guards).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect()
}

/// Every `.rs` under `src/` and `src/stdlib/`, as `(display path, text)`.
fn src_files() -> Vec<(String, String)> {
    let root = src_dir();
    let mut out = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(d) = dirs.pop() {
        let entries =
            std::fs::read_dir(&d).unwrap_or_else(|e| panic!("read_dir {}: {e}", d.display()));
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(root.parent().unwrap_or(Path::new("")))
                    .unwrap_or(&p)
                    .display()
                    .to_string();
                let text = std::fs::read_to_string(&p)
                    .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
                out.push((rel, text));
            }
        }
    }
    out.sort();
    assert!(
        out.len() > 40,
        "only {} source files walked out of src/ — the walker is stale",
        out.len()
    );
    out
}

/// The string literals in `s`, in order, with `\"` escapes respected.
fn literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '"' {
            let mut lit = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' && i + 1 < b.len() {
                    lit.push(b[i]);
                    i += 1;
                }
                lit.push(b[i]);
                i += 1;
            }
            out.push(lit);
        }
        i += 1;
    }
    out
}

/// A `const`/`static` name table: `(const name, 1-based line, entries)`.
///
/// Only bodies written out as a literal `&[..]` are returned — an alias
/// (`pub const CONSOLE_METHODS: &[&str] = METHODS;`) has no entries of its own
/// and is covered where its target is declared.
struct Table {
    file: String,
    line: usize,
    name: String,
    entries: Vec<String>,
}

fn str_tables() -> Vec<Table> {
    let mut out = Vec::new();
    for (file, text) in src_files() {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // `[pub] const|static NAME: &['static] [&['static] str] = <rhs>`
            let Some((decl, rhs)) = line.split_once('=') else {
                continue;
            };
            let d = decl.trim().trim_start_matches("pub ").trim();
            let Some(d) = d
                .strip_prefix("const ")
                .or_else(|| d.strip_prefix("static "))
            else {
                continue;
            };
            let Some((name, ty)) = d.split_once(':') else {
                continue;
            };
            let ty = ty.replace("'static", "");
            if ty.split_whitespace().collect::<String>() != "&[&str]" {
                continue;
            }
            if !rhs.trim_start().starts_with("&[") {
                continue; // alias, not a literal table
            }
            // Accumulate until the `&[..]` closes.
            let mut buf = String::from(rhs);
            let mut depth = rhs.matches('[').count() as i64 - rhs.matches(']').count() as i64;
            let mut j = i;
            while depth > 0 && j + 1 < lines.len() {
                j += 1;
                buf.push_str(lines[j]);
                depth += lines[j].matches('[').count() as i64;
                depth -= lines[j].matches(']').count() as i64;
            }
            out.push(Table {
                file: file.clone(),
                line: i + 1,
                name: name.trim().to_string(),
                entries: literals(&buf),
            });
        }
    }
    assert!(
        out.len() > 60,
        "only {} `&[&str]` name tables parsed out of src/ — the parser is stale",
        out.len()
    );
    out
}

/// A name table is a set, not a list: every consumer either inserts it key-by-key
/// into a map (`ensure_ctor_proto`, `ensure_native_protos`) or `contains`-tests it
/// (`is_method`, `instance_has_method`). A repeated entry is therefore either dead
/// text or — for the map consumers — an insert that overwrites the entry the first
/// occurrence made, and neither is ever what was meant.
#[test]
fn name_tables_have_no_duplicate_entries() {
    let mut problems = Vec::new();
    for t in str_tables() {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for e in &t.entries {
            *seen.entry(e.as_str()).or_default() += 1;
        }
        let dups: Vec<String> = seen
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(k, c)| format!("{k:?}\u{d7}{c}"))
            .collect();
        if !dups.is_empty() {
            problems.push(format!(
                "  {}:{} {} — {}",
                t.file,
                t.line,
                t.name,
                dups.join(", ")
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "duplicate entries in `&[&str]` name tables — each name is inserted into a \
         prototype/export map, where the later insert silently replaces the earlier:\n{}",
        problems.join("\n")
    );
}

/// The tag universe of `stdlib::instance_method_lists`: every string literal that
/// appears as a match pattern inside it, plus the `matches!` emitter list.
fn instance_tags() -> Vec<String> {
    let text =
        std::fs::read_to_string(src_dir().join("stdlib/mod.rs")).expect("read stdlib/mod.rs");
    let start = text
        .find("pub fn instance_method_lists")
        .expect("stdlib/mod.rs no longer defines `instance_method_lists` — parser is stale");
    // Walk to the end of the fn by brace depth.
    let body = &text[start..];
    let mut depth = 0i64;
    let mut end = body.len();
    let mut opened = false;
    for (idx, ch) in body.char_indices() {
        if ch == '{' {
            depth += 1;
            opened = true;
        } else if ch == '}' {
            depth -= 1;
            if opened && depth == 0 {
                end = idx;
                break;
            }
        }
    }
    let body = &body[..end];
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for line in body.lines() {
        let code = line.split("//").next().unwrap_or("");
        // Match arms (`"Tag" => ..`, `"A" | "B" => ..`) and the `matches!` list
        // (`| "Tag"`), but not the method names on an arm's right-hand side.
        let lhs = match code.split_once("=>") {
            Some((l, _)) => l,
            None if code.trim_start().starts_with('|') || code.trim_start().starts_with("tag,") => {
                code
            }
            None if code.trim().starts_with('"') && code.trim().ends_with(',') => "",
            None => "",
        };
        for lit in literals(lhs) {
            tags.insert(lit);
        }
    }
    let tags: Vec<String> = tags.into_iter().collect();
    assert!(
        tags.len() > 30,
        "only {} instance tags parsed out of `instance_method_lists` — parser is stale",
        tags.len()
    );
    tags
}

/// `host::ensure_ctor_proto` builds a native constructor's real `.prototype` by
/// looping `own.iter().chain(emitter.iter())` and `insert`-ing one property per
/// name. A name present in BOTH halves — or twice in one half — is inserted twice
/// into the same `IndexMap`, so the property's position in the key order is
/// decided by the first insert while its value comes from the last: exactly the
/// silent last-write-wins shape this file exists to catch, and invisible once the
/// map is built.
#[test]
fn instance_prototype_method_lists_are_disjoint_and_duplicate_free() {
    let mut problems = Vec::new();
    for tag in instance_tags() {
        let (own, emitter) = nodejs::stdlib::instance_method_lists(&tag);
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for m in own.iter().chain(emitter.iter()) {
            *counts.entry(*m).or_default() += 1;
        }
        let dups: Vec<&str> = counts
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(m, _)| *m)
            .collect();
        if !dups.is_empty() {
            let overlap: Vec<&str> = dups
                .iter()
                .filter(|m| own.contains(m) && emitter.contains(m))
                .copied()
                .collect();
            problems.push(format!(
                "  {tag}: {dups:?}{}",
                if overlap.is_empty() {
                    String::new()
                } else {
                    format!(" (own \u{2229} EventEmitter surface: {overlap:?})")
                }
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "a native prototype would be built by inserting the same method name twice \
         (`host::ensure_ctor_proto`), so one of the two registrations is dead:\n{}",
        problems.join("\n")
    );
}

/// Namespaces `stdlib::resolve` hands out, parsed from its match arms.
fn resolvable_namespaces() -> Vec<String> {
    let text =
        std::fs::read_to_string(src_dir().join("stdlib/mod.rs")).expect("read stdlib/mod.rs");
    let start = text
        .find("pub fn resolve(")
        .expect("stdlib/mod.rs no longer defines `resolve` — parser is stale");
    let mut out: BTreeSet<String> = BTreeSet::new();
    for line in text[start..].lines().skip(1) {
        if line.starts_with('}') {
            break;
        }
        let code = line.split("//").next().unwrap_or("");
        let Some((_, rhs)) = code.split_once("=>") else {
            continue;
        };
        for lit in literals(rhs) {
            out.insert(lit);
        }
    }
    let out: Vec<String> = out.into_iter().collect();
    assert!(
        out.len() > 40,
        "only {} namespaces parsed out of `stdlib::resolve` — parser is stale",
        out.len()
    );
    out
}

/// `resolve` is the *declaration* side of the module registry and
/// `namespace_methods`/`namespace_ctors` are the *installation* side, joined by a
/// bare string. A namespace declared on one side and missing on the other is the
/// name-keyed analogue of `opcode_ids.rs`'s "declared but never registered": the
/// `require` succeeds and yields a namespace object with no members at all, so the
/// failure surfaces later as `undefined is not a function` somewhere else.
#[test]
fn every_resolvable_namespace_has_members() {
    let empty: Vec<String> = resolvable_namespaces()
        .into_iter()
        .filter(|ns| !nodejs::stdlib::is_unimplemented(ns))
        .filter(|ns| nodejs::stdlib::namespace_keys(ns).is_empty())
        .collect();
    assert!(
        empty.is_empty(),
        "`stdlib::resolve` maps a specifier onto these namespaces but no method or \
         ctor table backs them, so `require` yields an empty object: {empty:?}"
    );
}

/// `namespace_keys` concatenates `namespace_ctors(ns)` and `namespace_methods(ns)`
/// into one key list, and `namespace_property` resolves a name against ONE of the
/// two. They are separate `match` statements over the same namespace, so nothing
/// stops a name being added to both — at which point the ctor half wins the
/// enumeration slot (it is emitted first) and the method the name was supposed to
/// reach becomes unenumerable-and-unreachable, or vice versa. The two tables must
/// be disjoint, and each must be duplicate-free in its own right.
#[test]
fn namespace_method_and_ctor_tables_are_disjoint() {
    let mut problems = Vec::new();
    for ns in resolvable_namespaces() {
        let methods = nodejs::stdlib::namespace_methods(&ns);
        let ctors = nodejs::stdlib::namespace_ctors(&ns);
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for m in methods.iter().chain(ctors.iter()) {
            *counts.entry(*m).or_default() += 1;
        }
        for (name, _) in counts.iter().filter(|(_, c)| **c > 1) {
            let both = methods.contains(name) && ctors.contains(name);
            problems.push(format!(
                "  {ns}.{name}{}",
                if both {
                    " (in namespace_methods AND namespace_ctors)"
                } else {
                    " (twice in one table)"
                }
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "a namespace member is registered twice; `namespace_keys` emits the ctor \
         table first, so the duplicate's other registration is unreachable:\n{}",
        problems.join("\n")
    );
}

/// Every member `namespace_keys` advertises must be one `is_method` will route,
/// and the reverse. They are built from the same two tables, so a divergence means
/// one of the tables grew an entry the join no longer sees.
#[test]
fn advertised_namespace_keys_are_all_dispatchable() {
    let mut problems = Vec::new();
    for ns in resolvable_namespaces() {
        for k in nodejs::stdlib::namespace_keys(&ns) {
            if !nodejs::stdlib::is_method(&format!("{ns}.{k}")) {
                problems.push(format!("  {ns}.{k}"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "enumerable namespace members that `is_method` will not dispatch:\n{}",
        problems.join("\n")
    );
}

/// `host::proto_for_name` reads `error_protos` FIRST and only then `native_protos`
/// — two independently populated `HashMap<String, Value>` sharing one name space.
/// A tag registered in both is reachable only through the error map, and the
/// native prototype built for it (with its methods and its `constructor`) is
/// unreachable. Nothing reports that; the object just silently lacks its methods.
#[test]
fn error_and_native_prototype_registries_do_not_share_a_name() {
    let host = std::fs::read_to_string(src_dir().join("host.rs")).expect("read host.rs");
    let error_names: Vec<String> = str_tables()
        .into_iter()
        .find(|t| t.name == "ERROR_NAMES")
        .expect("host.rs no longer declares ERROR_NAMES — parser is stale")
        .entries;
    assert!(
        host.contains("self.error_protos.insert"),
        "host.rs no longer populates `error_protos` by name — parser is stale"
    );

    // The `native_protos` name space: the element kinds plus the hand-named
    // prototypes `ensure_native_protos` installs, plus every tag
    // `ensure_ctor_proto` can install on demand.
    let mut native: BTreeSet<String> = ["Object", "TypedArray", "Buffer"]
        .into_iter()
        .map(str::to_string)
        .collect();
    for t in str_tables() {
        if t.name == "ELEMENT_KINDS" {
            native.extend(t.entries.iter().cloned());
        }
    }
    for tag in instance_tags() {
        let (own, emitter) = nodejs::stdlib::instance_method_lists(&tag);
        if !own.is_empty() || !emitter.is_empty() {
            native.insert(tag);
        }
    }

    let shared: Vec<&String> = error_names.iter().filter(|n| native.contains(*n)).collect();
    assert!(
        shared.is_empty(),
        "these names are registered in BOTH `error_protos` and `native_protos`; \
         `host::proto_for_name` prefers the error map, so the native prototype is \
         unreachable: {shared:?}"
    );
}
