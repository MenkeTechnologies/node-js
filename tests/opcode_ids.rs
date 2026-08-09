//! Structural guard on the hand-assigned builtin/opcode id space.
//!
//! Every JS operation the VM can't do natively is a `fusevm` builtin call whose
//! id is a hand-written `pub const` in `host.rs::ops`, dispatched through a
//! `vm.register_builtin(id, handler)` table in `builtins.rs`. Two concurrently
//! developed changes that each append a new op pick the same next number, the
//! two files merge without a conflict marker, and `register_builtin` keeps only
//! the LAST registration for a duplicated id — silently rerouting one operation
//! to the other's handler. Nothing in a normal build or run reports that.
//!
//! These tests read the constants back out of the source text (not out of the
//! compiled crate, where a duplicate is indistinguishable from an alias) and
//! fail on any duplicate value, any op that is declared but never registered,
//! any id registered twice, any two ops sharing one handler, and any builtin
//! call emitted from a bare integer instead of a named `ops::` constant.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn src(name: &str) -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src", name].iter().collect();
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Every `pub const NAME: TY = LITERAL;` inside `pub mod <module> { .. }`, keyed
/// by the module it was declared in. Brace depth is tracked so a nested block
/// never leaks constants into the wrong namespace.
fn consts_by_module(text: &str) -> BTreeMap<String, Vec<(String, String, String)>> {
    let mut out: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    let mut module: Option<String> = None;
    let mut depth = 0i32;
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or("");
        if module.is_none() {
            if let Some(rest) = code.trim().strip_prefix("pub mod ") {
                if let Some(name) = rest.strip_suffix(" {") {
                    module = Some(name.to_string());
                    depth = 1;
                    continue;
                }
            }
        }
        let Some(m) = module.clone() else { continue };
        if let Some(decl) = code.trim().strip_prefix("pub const ") {
            // `NAME: TY = LITERAL;`
            if let Some((name, rest)) = decl.split_once(':') {
                if let Some((ty, val)) = rest.split_once('=') {
                    let val = val.trim().trim_end_matches(';').trim();
                    out.entry(m.clone()).or_default().push((
                        name.trim().to_string(),
                        ty.trim().to_string(),
                        val.to_string(),
                    ));
                }
            }
        }
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
        if depth <= 0 {
            module = None;
        }
    }
    out
}

/// The `ops` namespace must exist and must be the u16 builtin id space.
fn op_consts() -> Vec<(String, String)> {
    let host = src("host.rs");
    let mods = consts_by_module(&host);
    let ops = mods
        .get("ops")
        .unwrap_or_else(|| panic!("host.rs no longer declares `pub mod ops`; parser is stale"));
    let ids: Vec<(String, String)> = ops
        .iter()
        .filter(|(_, ty, _)| ty == "u16")
        .map(|(n, _, v)| (n.clone(), v.clone()))
        .collect();
    assert!(
        ids.len() > 50,
        "only {} u16 op constants parsed out of host.rs::ops — parser is stale",
        ids.len()
    );
    ids
}

/// No two builtin ids may share a value: `register_builtin` keeps the last
/// writer, so a duplicate silently replaces a handler.
#[test]
fn builtin_op_ids_are_unique() {
    let mut by_value: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in op_consts() {
        by_value.entry(value).or_default().push(name);
    }
    let dups: Vec<String> = by_value
        .iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(v, names)| format!("  id {v} = {}", names.join(", ")))
        .collect();
    assert!(
        dups.is_empty(),
        "duplicate builtin ids in host.rs::ops — the later registration silently \
         replaces the earlier handler:\n{}",
        dups.join("\n")
    );
}

/// Every tag namespace (`unwind`, `member`, `binop`, `unop`, ...) is its own
/// hand-numbered space with the same collision hazard: a duplicate tag makes two
/// distinct operations decode to one branch of a `match`.
#[test]
fn tag_namespace_values_are_unique() {
    let host = src("host.rs");
    let mut problems = Vec::new();
    for (module, consts) in consts_by_module(&host) {
        let mut by_value: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for (name, ty, value) in consts {
            by_value.entry((ty, value)).or_default().push(name);
        }
        for ((ty, value), names) in by_value {
            if names.len() > 1 {
                problems.push(format!(
                    "  {module}: {ty} value {value} = {}",
                    names.join(", ")
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "duplicate tag values in host.rs namespaces:\n{}",
        problems.join("\n")
    );
}

/// `(op_const_name, handler_fn_name)` for each `vm.register_builtin(..)` line.
fn registrations() -> Vec<(String, String)> {
    let builtins = src("builtins.rs");
    let mut out = Vec::new();
    for line in builtins.lines() {
        let code = line.split("//").next().unwrap_or("");
        let Some(rest) = code.split_once("register_builtin(") else {
            continue;
        };
        let args = rest.1.trim_end().trim_end_matches(';').trim_end_matches(')');
        let Some((op, handler)) = args.split_once(',') else {
            panic!("unparsable register_builtin call: {line}");
        };
        let op = op.trim();
        let op = op.strip_prefix("ops::").unwrap_or_else(|| {
            panic!("register_builtin must take a named `ops::` constant, got `{op}`")
        });
        out.push((op.to_string(), handler.trim().to_string()));
    }
    assert!(
        !out.is_empty(),
        "no register_builtin calls parsed out of builtins.rs — parser is stale"
    );
    out
}

/// The declared id space and the dispatch table must be the same set: an op
/// declared but never registered dispatches to nothing, and an id registered
/// twice means one of the two handlers is dead.
#[test]
fn every_op_is_registered_exactly_once() {
    let declared: Vec<String> = op_consts().into_iter().map(|(n, _)| n).collect();
    let regs = registrations();

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (op, _) in &regs {
        *counts.entry(op.as_str()).or_default() += 1;
    }

    let missing: Vec<&String> = declared
        .iter()
        .filter(|n| !counts.contains_key(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "declared in host.rs::ops but never registered in builtins.rs: {missing:?}"
    );

    let twice: Vec<&str> = counts
        .iter()
        .filter(|(_, c)| **c > 1)
        .map(|(n, _)| *n)
        .collect();
    assert!(
        twice.is_empty(),
        "registered more than once (only the last handler survives): {twice:?}"
    );

    let unknown: Vec<&str> = counts
        .keys()
        .filter(|n| !declared.iter().any(|d| d == *n))
        .copied()
        .collect();
    assert!(
        unknown.is_empty(),
        "registered under an `ops::` name that host.rs::ops does not declare: {unknown:?}"
    );
}

/// Two ops pointing at one handler is the shape a copy-pasted registration
/// takes when the op name was updated but the handler name was not.
#[test]
fn each_handler_serves_one_op() {
    let mut by_handler: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (op, handler) in registrations() {
        by_handler.entry(handler).or_default().push(op);
    }
    let shared: Vec<String> = by_handler
        .iter()
        .filter(|(_, ops)| ops.len() > 1)
        .map(|(h, ops)| format!("  {h} <- {}", ops.join(", ")))
        .collect();
    assert!(
        shared.is_empty(),
        "one handler registered for several ops:\n{}",
        shared.join("\n")
    );
}

/// A bare integer in `Op::CallBuiltin(7, ..)` bypasses the named id space
/// entirely, so neither the uniqueness test above nor a human reading `ops` can
/// see it. Every emitted builtin call must name its constant.
#[test]
fn compiler_emits_only_named_op_constants() {
    let compiler = src("compiler.rs");
    let mut bare = Vec::new();
    for (i, line) in compiler.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let Some((_, rest)) = code.split_once("Op::CallBuiltin(") else {
            continue;
        };
        let first = rest.split(',').next().unwrap_or("").trim();
        if first.starts_with(|c: char| c.is_ascii_digit()) {
            bare.push(format!("  compiler.rs:{}: {}", i + 1, line.trim()));
        }
    }
    assert!(
        bare.is_empty(),
        "builtin call emitted from a bare integer instead of an `ops::` constant:\n{}",
        bare.join("\n")
    );
}
