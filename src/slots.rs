//! Which locals can live in fusevm frame slots instead of the host's scope
//! chain.
//!
//! Every identifier in a node-js program is a name lookup: the compiler emits
//! `CallBuiltin(GETLOCAL)` with the name as a string constant, and the host pops
//! it off the VM stack, borrows the thread-local `JsHost`, walks the
//! `Rc<RefCell<EnvData>>` chain and hashes the name in each scope. An empty
//! `for (let i = 0; i < 5_000_000; i++) {}` costs four of those round-trips per
//! iteration — 178 ns against node's 6 ns — and none of them is JIT-able, since
//! fusevm's block tier declines any region containing a `CallBuiltin`.
//!
//! A local that no closure can reach does not need a scope entry at all: it can
//! live in the frame slot vector fusevm already keeps, addressed by index
//! (`Op::GetSlot` / `Op::SetSlot`). This module decides which names qualify.
//! The shape of the analysis follows pythonrs's `fn_slots_allowed` /
//! `fn_slots`, which does the same job against the same VM.
//!
//! The rules are deliberately conservative, because a name that is slotted in
//! one place and looked up by name in another is a silent wrong answer:
//!
//! 1. **A name reachable from another chunk keeps its binding.** A nested
//!    function, arrow or class body compiles to its own chunk and resolves what
//!    it captures through the environment chain; a `try` block is likewise its
//!    own chunk on its own VM frame, so a slot written outside it is invisible
//!    inside. Every identifier mentioned in either is therefore off the table —
//!    but only those identifiers, not the whole chunk, so a loop counter still
//!    gets a slot in a file that also defines a callback. A direct `eval` can
//!    name anything, so it disables the chunk outright.
//! 2. **One declaration per name.** Shadowing (`let x` in two sibling blocks) is
//!    exactly where a flat name→slot table would be wrong, so a name declared
//!    more than once in the chunk is left alone rather than scope-tracked.
//! 3. **No read before the declaration, in source order.** This is what keeps
//!    the temporal dead zone and `var` hoisting behaving as they do today: a
//!    slot reads as `undefined` before its first write, which is neither node's
//!    `ReferenceError` for a `let` nor what node-js currently answers. A name
//!    whose first mention is a read stays on the name path, where those answers
//!    come from. Source order is a conservative stand-in for execution order —
//!    a loop can re-enter a block, but it cannot reach a statement the source
//!    has not introduced yet.
//! 4. **Simple identifiers only.** Destructuring targets, `delete x`, and
//!    anything that is not an `Expr::Ident` bind through the host.
//! 5. **At the top level of a script, `let`/`const` only.** A top-level `var` is
//!    a property of the global object (`var g = 1; globalThis.g` is `1` on node
//!    v26.7.0), so it has to stay a real binding.

use crate::ast::{DeclKind, Expr, Param, Prop, Stmt, StmtKind, SwitchCase};
use rustc_hash::{FxHashMap, FxHashSet};

/// Name → frame slot for one chunk. Empty when the chunk is not eligible.
pub type SlotTable = FxHashMap<String, u16>;

/// What the compiler needs to know about a chunk's locals.
#[derive(Default)]
pub struct Plan {
    /// The slotted names and their frame indices.
    pub table: SlotTable,
    /// Of those, the ones provably holding a Number, so `++`/`--` can be a
    /// native add instead of a `NUM_STEP` call into the host.
    pub numeric: FxHashSet<String>,
}

/// Is this initializer a literal Number? `-1` reaches the compiler as a unary
/// negation of `1`, so it counts too.
fn is_number_literal(e: &Expr) -> bool {
    match e {
        Expr::Number(_) => true,
        Expr::Unary(crate::ast::UnOp::Neg, inner) => matches!(**inner, Expr::Number(_)),
        _ => false,
    }
}

/// Plan the slots for a chunk: `params` are bound into the environment by the
/// caller before the chunk runs (the compiler emits a prologue that copies each
/// into its slot), `body` is the statement list about to be compiled, and
/// `top_level` marks a script/module body, where `var` stays a global.
pub fn plan(params: &[Param], body: &[Stmt], top_level: bool) -> Plan {
    if !chunk_is_eligible(body) {
        return Plan::default();
    }
    // Anything a nested chunk can name resolves through the environment, so it
    // cannot move into this frame's slots.
    let mut escaping = FxHashSet::default();
    for s in body {
        collect_escaping_stmt(s, &mut escaping);
    }
    let mut p = Planner {
        candidates: SlotTable::default(),
        rejected: escaping,
        numeric: FxHashSet::default(),
        top_level,
        next: 0,
    };
    // Parameters are declared and assigned before the first statement runs.
    // Their incoming type is whatever the caller passed, so they are never in
    // the numeric set.
    for name in param_names(params) {
        p.declare(&name);
    }
    for s in body {
        p.walk_stmt(s);
    }
    for name in p.rejected {
        p.candidates.remove(&name);
        p.numeric.remove(&name);
    }
    Plan {
        table: p.candidates,
        numeric: p.numeric,
    }
}

/// The simple identifier parameters, in order. A destructuring or rest pattern
/// is bound by the body prologue, so it is not seeded here.
pub fn param_names(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter(|p| !p.rest)
        .filter_map(|p| match &p.pattern {
            Expr::Ident(n) => Some(n.clone()),
            _ => None,
        })
        .collect()
}

/// Is this chunk's frame stable enough to hold slots at all? Two things say no
/// for the whole chunk rather than for one name: a direct `eval`, which can read
/// or write any binding by a name only known at run time, and a `yield`/`await`
/// at this chunk's own level, which suspends the frame the slots live in.
/// (Nested function bodies are their own chunks; what they contain is their
/// business, and what they NAME is handled per-name by `collect_escaping_*`.)
fn chunk_is_eligible(body: &[Stmt]) -> bool {
    !mentions_eval_stmts(body) && body.iter().all(stmt_slot_safe)
}

/// `eval` named anywhere, at any depth — including inside a nested function,
/// which can be handed this frame's environment.
fn mentions_eval_stmts(body: &[Stmt]) -> bool {
    let mut names = FxHashSet::default();
    for s in body {
        collect_all_idents_stmt(s, &mut names);
    }
    names.contains("eval")
}

/// A `try` runs as its own chunk on its own frame; `yield`/`await` suspend the
/// frame; `delete x` removes a binding a slot does not have.
fn stmt_slot_safe(s: &Stmt) -> bool {
    match &s.kind {
        // A `try` compiles to sub-chunks; the names they touch are handled by
        // `collect_escaping_stmt`, so the statement itself is no obstacle.
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            block.iter().all(stmt_slot_safe)
                && handler
                    .as_ref()
                    .map_or(true, |(_, b)| b.iter().all(stmt_slot_safe))
                && finalizer
                    .as_ref()
                    .map_or(true, |b| b.iter().all(stmt_slot_safe))
        }
        StmtKind::Expr(e) | StmtKind::Throw(e) => expr_slot_safe(e),
        StmtKind::Return(e) => e.as_ref().map_or(true, expr_slot_safe),
        StmtKind::Decl { decls, .. } => decls
            .iter()
            .all(|d| d.init.as_ref().map_or(true, expr_slot_safe)),
        StmtKind::Block(body) => body.iter().all(stmt_slot_safe),
        StmtKind::If { test, cons, alt } => {
            expr_slot_safe(test)
                && stmt_slot_safe(cons)
                && alt.as_deref().map_or(true, stmt_slot_safe)
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            expr_slot_safe(test) && stmt_slot_safe(body)
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            init.as_deref().map_or(true, stmt_slot_safe)
                && test.as_ref().map_or(true, expr_slot_safe)
                && update.as_ref().map_or(true, expr_slot_safe)
                && stmt_slot_safe(body)
        }
        StmtKind::ForOf {
            target,
            iter,
            body,
            is_await,
            ..
        } => !*is_await && expr_slot_safe(target) && expr_slot_safe(iter) && stmt_slot_safe(body),
        StmtKind::ForIn {
            target,
            object,
            body,
            ..
        } => expr_slot_safe(target) && expr_slot_safe(object) && stmt_slot_safe(body),
        StmtKind::Switch { disc, cases } => {
            expr_slot_safe(disc)
                && cases.iter().all(|c: &SwitchCase| {
                    c.test.as_ref().map_or(true, expr_slot_safe)
                        && c.body.iter().all(stmt_slot_safe)
                })
        }
        StmtKind::Labeled { body, .. } => stmt_slot_safe(body),
        StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => true,
        // A nested function or class body is its own chunk with its own frame:
        // nothing in it can unsettle this one. What it NAMES is refused per
        // name by `collect_escaping_stmt`.
        StmtKind::FuncDecl { .. } | StmtKind::ClassDecl(_) => true,
    }
}

fn expr_slot_safe(e: &Expr) -> bool {
    let all = |xs: &[Expr]| xs.iter().all(expr_slot_safe);
    match e {
        Expr::Yield { .. } | Expr::Await(_) => false,
        // `delete x` is a binding operation a slot cannot express — the NAME is
        // refused in the walk, the statement holding it is fine.
        Expr::Unary(_, inner) | Expr::Spread(inner) => expr_slot_safe(inner),
        Expr::Template { exprs, .. } => all(exprs),
        Expr::TaggedTemplate { tag, exprs, .. } => expr_slot_safe(tag) && all(exprs),
        Expr::Array(items) | Expr::Sequence(items) => all(items),
        Expr::Object(props) => props.iter().all(|p| match p {
            Prop::KeyValue { key, value, .. } => expr_slot_safe(key) && expr_slot_safe(value),
            Prop::Spread(x) => expr_slot_safe(x),
            // An accessor is a nested function: its names escape, it is not a
            // reason to give up on the chunk.
            Prop::Accessor { .. } => true,
        }),
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => expr_slot_safe(l) && expr_slot_safe(r),
        Expr::Conditional { test, cons, alt } => {
            expr_slot_safe(test) && expr_slot_safe(cons) && expr_slot_safe(alt)
        }
        Expr::Assign { target, value } => expr_slot_safe(target) && expr_slot_safe(value),
        Expr::Update { target, .. } => expr_slot_safe(target),
        Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
            expr_slot_safe(func) && all(args)
        }
        Expr::Member { object, .. } => expr_slot_safe(object),
        Expr::Index { object, index, .. } => expr_slot_safe(object) && expr_slot_safe(index),
        // Its own chunk, its own frame; `collect_escaping_expr` takes the names.
        Expr::Function { .. } | Expr::Class(_) => true,
        _ => true,
    }
}

struct Planner {
    candidates: SlotTable,
    rejected: FxHashSet<String>,
    /// Slots whose value is provably a JS Number at every point: declared from
    /// a numeric literal and written afterwards only by `++`/`--`, which on a
    /// Number yields a Number. `i++` on one of these needs no `NUM_STEP` call
    /// into the host to do `ToNumeric` and stay BigInt-aware.
    numeric: FxHashSet<String>,
    top_level: bool,
    next: u16,
}

impl Planner {
    /// A declaration in source order: the first one claims a slot, a second one
    /// (rule 2) gives the name back to the host.
    fn declare(&mut self, name: &str) {
        if self.rejected.contains(name) {
            return;
        }
        if self.candidates.contains_key(name) {
            self.reject(name);
            return;
        }
        // fusevm addresses slots with a `u16`; a chunk with more locals than
        // that keeps the rest on the name path.
        if self.next == u16::MAX {
            self.reject(name);
            return;
        }
        self.candidates.insert(name.to_string(), self.next);
        self.next += 1;
    }

    fn reject(&mut self, name: &str) {
        self.rejected.insert(name.to_string());
    }

    /// A mention of `name` that is not its declaration (rule 3): if the name has
    /// no slot yet, its first mention is a read, so it never gets one.
    fn mention(&mut self, name: &str) {
        if !self.candidates.contains_key(name) {
            self.reject(name);
        }
    }

    /// The declaration target of a `let`/`var`/`for`-head binding.
    fn declare_target(&mut self, target: &Expr, kind: Option<DeclKind>) {
        self.declare_target_init(target, kind, None)
    }

    /// As `declare_target`, plus the initializer, which decides whether the
    /// binding starts out a Number (see `Planner::numeric`).
    fn declare_target_init(&mut self, target: &Expr, kind: Option<DeclKind>, init: Option<&Expr>) {
        let Expr::Ident(n) = target else {
            // A destructuring pattern binds through the host; every name in it
            // is off the table.
            self.reject_names_in(target);
            return;
        };
        match kind {
            // Rule 5: a top-level `var` is a global-object property.
            Some(DeclKind::Var) if self.top_level => self.reject(n),
            Some(_) => {
                self.declare(n);
                if init.is_some_and(is_number_literal) {
                    self.numeric.insert(n.clone());
                }
            }
            // `for (x of …)` with no declaration keyword assigns an existing
            // binding — that is a mention, not a declaration.
            None => self.mention(n),
        }
    }

    fn reject_names_in(&mut self, e: &Expr) {
        let mut names = Vec::new();
        collect_idents(e, &mut names);
        for n in names {
            self.reject(&n);
        }
    }

    fn walk_stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Decl { kind, decls } => {
                for d in decls {
                    // The initializer is evaluated BEFORE the binding exists.
                    if let Some(init) = &d.init {
                        self.walk_expr(init);
                    }
                    match &d.init {
                        // `let x;` with no initializer leaves the binding
                        // unassigned, which is exactly the read-before-write
                        // case slots cannot answer for.
                        None => self.reject_names_in(&d.target),
                        Some(init) => self.declare_target_init(&d.target, Some(*kind), Some(init)),
                    }
                }
            }
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.walk_expr(e),
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.walk_expr(e);
                }
            }
            StmtKind::Block(body) => {
                for s in body {
                    self.walk_stmt(s);
                }
            }
            StmtKind::If { test, cons, alt } => {
                self.walk_expr(test);
                self.walk_stmt(cons);
                if let Some(alt) = alt {
                    self.walk_stmt(alt);
                }
            }
            StmtKind::While { test, body } => {
                self.walk_expr(test);
                self.walk_stmt(body);
            }
            StmtKind::DoWhile { body, test } => {
                self.walk_stmt(body);
                self.walk_expr(test);
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                if let Some(init) = init {
                    self.walk_stmt(init);
                }
                if let Some(test) = test {
                    self.walk_expr(test);
                }
                self.walk_stmt(body);
                if let Some(update) = update {
                    self.walk_expr(update);
                }
            }
            StmtKind::ForOf {
                decl_kind,
                target,
                iter,
                body,
                ..
            } => {
                self.walk_expr(iter);
                self.declare_target(target, *decl_kind);
                self.walk_stmt(body);
            }
            StmtKind::ForIn {
                decl_kind,
                target,
                object,
                body,
            } => {
                self.walk_expr(object);
                self.declare_target(target, *decl_kind);
                self.walk_stmt(body);
            }
            StmtKind::Switch { disc, cases } => {
                self.walk_expr(disc);
                for c in cases {
                    if let Some(t) = &c.test {
                        self.walk_expr(t);
                    }
                    for s in &c.body {
                        self.walk_stmt(s);
                    }
                }
            }
            StmtKind::Labeled { body, .. } => self.walk_stmt(body),
            // Refused by `chunk_is_eligible` before the walk starts.
            StmtKind::Try { .. } | StmtKind::FuncDecl { .. } | StmtKind::ClassDecl(_) => {}
            StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => {}
        }
    }

    fn walk_expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(n) => self.mention(n),
            // An assignment to a name that has no slot yet is a plain store to
            // whatever binding exists — a mention, not a declaration.
            Expr::Assign { target, value } => {
                self.walk_expr(value);
                match &**target {
                    // A plain store of an arbitrary value: whatever the slot
                    // held, it is not provably a Number after this.
                    Expr::Ident(n) => {
                        self.numeric.remove(n);
                        self.mention(n);
                    }
                    other => self.walk_expr(other),
                }
            }
            Expr::Update { target, .. } => self.walk_expr(target),
            // `delete x` asks the host to remove a binding; a slot has none.
            Expr::Unary(crate::ast::UnOp::Delete, inner) => {
                if let Expr::Ident(n) = &**inner {
                    self.reject(n);
                } else {
                    self.walk_expr(inner);
                }
            }
            Expr::Unary(_, inner) | Expr::Spread(inner) | Expr::Await(inner) => {
                self.walk_expr(inner)
            }
            Expr::Yield { arg: Some(a), .. } => self.walk_expr(a),
            Expr::Template { exprs, .. } => self.walk_all(exprs),
            Expr::TaggedTemplate { tag, exprs, .. } => {
                self.walk_expr(tag);
                self.walk_all(exprs);
            }
            Expr::Array(items) | Expr::Sequence(items) => self.walk_all(items),
            Expr::Object(props) => {
                for p in props {
                    match p {
                        Prop::KeyValue { key, value, .. } => {
                            self.walk_expr(key);
                            self.walk_expr(value);
                        }
                        Prop::Spread(x) => self.walk_expr(x),
                        Prop::Accessor { key, func, .. } => {
                            self.walk_expr(key);
                            self.walk_expr(func);
                        }
                    }
                }
            }
            Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => {
                self.walk_expr(l);
                self.walk_expr(r);
            }
            Expr::Conditional { test, cons, alt } => {
                self.walk_expr(test);
                self.walk_expr(cons);
                self.walk_expr(alt);
            }
            Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
                self.walk_expr(func);
                self.walk_all(args);
            }
            Expr::Member { object, .. } => self.walk_expr(object),
            Expr::Index { object, index, .. } => {
                self.walk_expr(object);
                self.walk_expr(index);
            }
            // A nested function or class body is refused by `chunk_is_eligible`,
            // so anything it names is already out of reach here.
            Expr::Function { .. } | Expr::Class(_) => {}
            _ => {}
        }
    }

    fn walk_all(&mut self, items: &[Expr]) {
        for e in items {
            self.walk_expr(e);
        }
    }
}

// ── names another chunk can reach ────────────────────────────────────────────
//
// A nested function, arrow, class body or `try` part compiles to a chunk of its
// own and resolves every name it uses through the environment chain. Whatever
// those chunks mention therefore has to stay a real binding — and only that,
// which is what lets a counting loop keep its slots in a file that also passes
// a callback to `map`.

fn collect_escaping_stmt(s: &Stmt, out: &mut FxHashSet<String>) {
    match &s.kind {
        // The declaration's own name is bound by the hoisting pass, by name.
        StmtKind::FuncDecl { name, .. } => {
            out.insert(name.clone());
            collect_all_idents_stmt(s, out);
        }
        StmtKind::ClassDecl(c) => {
            if let Some(n) = &c.name {
                out.insert(n.clone());
            }
            collect_all_idents_stmt(s, out);
        }
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            for st in block {
                collect_all_idents_stmt(st, out);
            }
            if let Some((bind, body)) = handler {
                if let Some(p) = bind {
                    collect_all_idents_expr(p, out);
                }
                for st in body {
                    collect_all_idents_stmt(st, out);
                }
            }
            if let Some(body) = finalizer {
                for st in body {
                    collect_all_idents_stmt(st, out);
                }
            }
        }
        StmtKind::Expr(e) | StmtKind::Throw(e) => collect_escaping_expr(e, out),
        StmtKind::Return(e) => {
            if let Some(e) = e {
                collect_escaping_expr(e, out);
            }
        }
        StmtKind::Decl { decls, .. } => {
            for d in decls {
                if let Some(init) = &d.init {
                    collect_escaping_expr(init, out);
                }
            }
        }
        StmtKind::Block(body) => {
            for st in body {
                collect_escaping_stmt(st, out);
            }
        }
        StmtKind::If { test, cons, alt } => {
            collect_escaping_expr(test, out);
            collect_escaping_stmt(cons, out);
            if let Some(alt) = alt {
                collect_escaping_stmt(alt, out);
            }
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            collect_escaping_expr(test, out);
            collect_escaping_stmt(body, out);
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            if let Some(init) = init {
                collect_escaping_stmt(init, out);
            }
            if let Some(test) = test {
                collect_escaping_expr(test, out);
            }
            if let Some(update) = update {
                collect_escaping_expr(update, out);
            }
            collect_escaping_stmt(body, out);
        }
        StmtKind::ForOf {
            target, iter, body, ..
        } => {
            collect_escaping_expr(target, out);
            collect_escaping_expr(iter, out);
            collect_escaping_stmt(body, out);
        }
        StmtKind::ForIn {
            target,
            object,
            body,
            ..
        } => {
            collect_escaping_expr(target, out);
            collect_escaping_expr(object, out);
            collect_escaping_stmt(body, out);
        }
        StmtKind::Switch { disc, cases } => {
            collect_escaping_expr(disc, out);
            for c in cases {
                if let Some(t) = &c.test {
                    collect_escaping_expr(t, out);
                }
                for st in &c.body {
                    collect_escaping_stmt(st, out);
                }
            }
        }
        StmtKind::Labeled { body, .. } => collect_escaping_stmt(body, out),
        StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => {}
    }
}

fn collect_escaping_expr(e: &Expr, out: &mut FxHashSet<String>) {
    match e {
        // Here is the boundary: everything named inside a nested function or
        // class body is resolved from the environment when that chunk runs.
        Expr::Function { .. } | Expr::Class(_) => collect_all_idents_expr(e, out),
        Expr::Ident(_) | Expr::Null | Expr::Undefined | Expr::True | Expr::False => {}
        Expr::Unary(_, x) | Expr::Spread(x) | Expr::Await(x) | Expr::Member { object: x, .. } => {
            collect_escaping_expr(x, out)
        }
        Expr::Yield { arg: Some(a), .. } => collect_escaping_expr(a, out),
        Expr::Template { exprs, .. } => exprs.iter().for_each(|x| collect_escaping_expr(x, out)),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            collect_escaping_expr(tag, out);
            exprs.iter().for_each(|x| collect_escaping_expr(x, out));
        }
        Expr::Array(items) | Expr::Sequence(items) => {
            items.iter().for_each(|x| collect_escaping_expr(x, out))
        }
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::KeyValue { key, value, .. } => {
                        collect_escaping_expr(key, out);
                        collect_escaping_expr(value, out);
                    }
                    Prop::Spread(x) => collect_escaping_expr(x, out),
                    Prop::Accessor { key, func, .. } => {
                        collect_escaping_expr(key, out);
                        collect_all_idents_expr(func, out);
                    }
                }
            }
        }
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => {
            collect_escaping_expr(l, out);
            collect_escaping_expr(r, out);
        }
        Expr::Conditional { test, cons, alt } => {
            collect_escaping_expr(test, out);
            collect_escaping_expr(cons, out);
            collect_escaping_expr(alt, out);
        }
        Expr::Assign { target, value } => {
            collect_escaping_expr(target, out);
            collect_escaping_expr(value, out);
        }
        Expr::Update { target, .. } => collect_escaping_expr(target, out),
        Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
            collect_escaping_expr(func, out);
            args.iter().for_each(|x| collect_escaping_expr(x, out));
        }
        Expr::Index { object, index, .. } => {
            collect_escaping_expr(object, out);
            collect_escaping_expr(index, out);
        }
        _ => {}
    }
}

/// Every identifier anywhere in a statement, nested bodies included.
fn collect_all_idents_stmt(s: &Stmt, out: &mut FxHashSet<String>) {
    match &s.kind {
        StmtKind::FuncDecl { params, body, .. } => {
            for p in params {
                collect_all_idents_expr(&p.pattern, out);
                if let Some(d) = &p.default {
                    collect_all_idents_expr(d, out);
                }
            }
            body.iter().for_each(|st| collect_all_idents_stmt(st, out));
        }
        StmtKind::ClassDecl(c) => collect_class_idents(c, out),
        StmtKind::Expr(e) | StmtKind::Throw(e) => collect_all_idents_expr(e, out),
        StmtKind::Return(e) => {
            if let Some(e) = e {
                collect_all_idents_expr(e, out);
            }
        }
        StmtKind::Decl { decls, .. } => {
            for d in decls {
                collect_all_idents_expr(&d.target, out);
                if let Some(init) = &d.init {
                    collect_all_idents_expr(init, out);
                }
            }
        }
        StmtKind::Block(body) => body.iter().for_each(|st| collect_all_idents_stmt(st, out)),
        StmtKind::If { test, cons, alt } => {
            collect_all_idents_expr(test, out);
            collect_all_idents_stmt(cons, out);
            if let Some(alt) = alt {
                collect_all_idents_stmt(alt, out);
            }
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            collect_all_idents_expr(test, out);
            collect_all_idents_stmt(body, out);
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            if let Some(init) = init {
                collect_all_idents_stmt(init, out);
            }
            if let Some(test) = test {
                collect_all_idents_expr(test, out);
            }
            if let Some(update) = update {
                collect_all_idents_expr(update, out);
            }
            collect_all_idents_stmt(body, out);
        }
        StmtKind::ForOf {
            target, iter, body, ..
        } => {
            collect_all_idents_expr(target, out);
            collect_all_idents_expr(iter, out);
            collect_all_idents_stmt(body, out);
        }
        StmtKind::ForIn {
            target,
            object,
            body,
            ..
        } => {
            collect_all_idents_expr(target, out);
            collect_all_idents_expr(object, out);
            collect_all_idents_stmt(body, out);
        }
        StmtKind::Switch { disc, cases } => {
            collect_all_idents_expr(disc, out);
            for c in cases {
                if let Some(t) = &c.test {
                    collect_all_idents_expr(t, out);
                }
                c.body
                    .iter()
                    .for_each(|st| collect_all_idents_stmt(st, out));
            }
        }
        StmtKind::Labeled { body, .. } => collect_all_idents_stmt(body, out),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            block.iter().for_each(|st| collect_all_idents_stmt(st, out));
            if let Some((bind, body)) = handler {
                if let Some(p) = bind {
                    collect_all_idents_expr(p, out);
                }
                body.iter().for_each(|st| collect_all_idents_stmt(st, out));
            }
            if let Some(body) = finalizer {
                body.iter().for_each(|st| collect_all_idents_stmt(st, out));
            }
        }
        StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => {}
    }
}

fn collect_class_idents(c: &crate::ast::ClassNode, out: &mut FxHashSet<String>) {
    if let Some(p) = &c.parent {
        collect_all_idents_expr(p, out);
    }
    for m in &c.members {
        collect_all_idents_expr(&m.key, out);
        for p in &m.params {
            collect_all_idents_expr(&p.pattern, out);
            if let Some(d) = &p.default {
                collect_all_idents_expr(d, out);
            }
        }
        m.body
            .iter()
            .for_each(|st| collect_all_idents_stmt(st, out));
        if let Some(init) = &m.field_init {
            collect_all_idents_expr(init, out);
        }
    }
}

fn collect_all_idents_expr(e: &Expr, out: &mut FxHashSet<String>) {
    let all = |xs: &[Expr], out: &mut FxHashSet<String>| {
        xs.iter().for_each(|x| collect_all_idents_expr(x, out))
    };
    match e {
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        Expr::Class(c) => collect_class_idents(c, out),
        Expr::Function { params, body, .. } => {
            for p in params {
                collect_all_idents_expr(&p.pattern, out);
                if let Some(d) = &p.default {
                    collect_all_idents_expr(d, out);
                }
            }
            match body {
                crate::ast::FnBody::Block(stmts) => {
                    stmts.iter().for_each(|st| collect_all_idents_stmt(st, out))
                }
                crate::ast::FnBody::Expr(x) => collect_all_idents_expr(x, out),
            }
        }
        Expr::Unary(_, x) | Expr::Spread(x) | Expr::Await(x) | Expr::Member { object: x, .. } => {
            collect_all_idents_expr(x, out)
        }
        Expr::Yield { arg: Some(a), .. } => collect_all_idents_expr(a, out),
        Expr::Template { exprs, .. } => all(exprs, out),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            collect_all_idents_expr(tag, out);
            all(exprs, out);
        }
        Expr::Array(items) | Expr::Sequence(items) => all(items, out),
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::KeyValue { key, value, .. } => {
                        collect_all_idents_expr(key, out);
                        collect_all_idents_expr(value, out);
                    }
                    Prop::Spread(x) => collect_all_idents_expr(x, out),
                    Prop::Accessor { key, func, .. } => {
                        collect_all_idents_expr(key, out);
                        collect_all_idents_expr(func, out);
                    }
                }
            }
        }
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => {
            collect_all_idents_expr(l, out);
            collect_all_idents_expr(r, out);
        }
        Expr::Conditional { test, cons, alt } => {
            collect_all_idents_expr(test, out);
            collect_all_idents_expr(cons, out);
            collect_all_idents_expr(alt, out);
        }
        Expr::Assign { target, value } => {
            collect_all_idents_expr(target, out);
            collect_all_idents_expr(value, out);
        }
        Expr::Update { target, .. } => collect_all_idents_expr(target, out),
        Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
            collect_all_idents_expr(func, out);
            all(args, out);
        }
        Expr::Index { object, index, .. } => {
            collect_all_idents_expr(object, out);
            collect_all_idents_expr(index, out);
        }
        _ => {}
    }
}

/// Every identifier appearing in a (possibly destructuring) target expression.
fn collect_idents(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ident(n) => out.push(n.clone()),
        Expr::Array(items) | Expr::Sequence(items) => {
            for x in items {
                collect_idents(x, out);
            }
        }
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::KeyValue { value, .. } => collect_idents(value, out),
                    Prop::Spread(x) => collect_idents(x, out),
                    Prop::Accessor { .. } => {}
                }
            }
        }
        Expr::Spread(inner) => collect_idents(inner, out),
        Expr::Assign { target, .. } => collect_idents(target, out),
        _ => {}
    }
}
