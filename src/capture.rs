//! Does a piece of a program hold on to the scope it runs in?
//!
//! Two lowerings in `compiler.rs` exist only to serve code that captures an
//! environment, and both cost a heap allocation every time control passes them:
//!
//! * a `for (let i = …)` head is re-bound per iteration (ForBodyEvaluation's
//!   CreatePerIterationEnvironment), so a closure made in one pass keeps that
//!   pass's value — `COPY_SCOPE` clones the whole scope on every iteration;
//! * a `{ … }` block opens a scope for its lexical declarations.
//!
//! When nothing in the subtree can make a closure, the per-iteration copy is
//! unobservable: no one can ever hold a reference to the iteration's bindings,
//! so one binding mutated in place gives the same answers. A profile of
//! `for (let i = 0; i < 5_000_000; i++) s += i % 7;` spent 17% of its samples in
//! `copy_scope` and the `EnvData` allocate/free traffic under it, for copies
//! that nothing could observe.
//!
//! This module answers the question conservatively: it says "captures" for
//! anything that makes a function, a class (its methods are functions), or a
//! direct `eval` (which can both make closures and declare into the caller's
//! scope). Every match here is exhaustive — a new AST node has to be classified
//! deliberately rather than defaulting into the fast path.

use crate::ast::{Expr, Prop, Stmt, StmtKind, SwitchCase};

/// True if evaluating `s` can create something that outlives it holding the
/// current scope.
pub fn stmt_captures(s: &Stmt) -> bool {
    match &s.kind {
        // A function or class body is exactly the thing that captures.
        StmtKind::FuncDecl { .. } | StmtKind::ClassDecl(_) => true,

        StmtKind::Expr(e) | StmtKind::Throw(e) => expr_captures(e),
        StmtKind::Return(e) => e.as_ref().is_some_and(expr_captures),
        StmtKind::Decl { decls, .. } => decls
            .iter()
            .any(|d| expr_captures(&d.target) || d.init.as_ref().is_some_and(expr_captures)),
        StmtKind::Block(body) => body.iter().any(stmt_captures),
        StmtKind::If { test, cons, alt } => {
            expr_captures(test)
                || stmt_captures(cons)
                || alt.as_deref().is_some_and(stmt_captures)
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            expr_captures(test) || stmt_captures(body)
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            init.as_deref().is_some_and(stmt_captures)
                || test.as_ref().is_some_and(expr_captures)
                || update.as_ref().is_some_and(expr_captures)
                || stmt_captures(body)
        }
        StmtKind::ForOf {
            target, iter, body, ..
        } => expr_captures(target) || expr_captures(iter) || stmt_captures(body),
        StmtKind::ForIn {
            target,
            object,
            body,
            ..
        } => expr_captures(target) || expr_captures(object) || stmt_captures(body),
        StmtKind::Switch { disc, cases } => {
            expr_captures(disc) || cases.iter().any(case_captures)
        }
        StmtKind::Labeled { body, .. } => stmt_captures(body),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            block.iter().any(stmt_captures)
                || handler.as_ref().is_some_and(|(param, body)| {
                    param.as_ref().is_some_and(expr_captures) || body.iter().any(stmt_captures)
                })
                || finalizer
                    .as_ref()
                    .is_some_and(|body| body.iter().any(stmt_captures))
        }
        StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => false,
    }
}

fn case_captures(c: &SwitchCase) -> bool {
    c.test.as_ref().is_some_and(expr_captures) || c.body.iter().any(stmt_captures)
}

/// True if evaluating `e` can create something that outlives it holding the
/// current scope.
pub fn expr_captures(e: &Expr) -> bool {
    match e {
        // The three that capture.
        Expr::Function { .. } | Expr::Class(_) => true,
        // Direct `eval` sees — and can close over, or declare into — the scope
        // it is called from, so a scope it can reach must stay a real scope.
        // Any other use of the name (`const f = eval;`) is caught here too:
        // the check is on the identifier, not on the call shape.
        Expr::Ident(n) => n == "eval",

        Expr::Null
        | Expr::Undefined
        | Expr::True
        | Expr::False
        | Expr::Number(_)
        | Expr::BigInt(_)
        | Expr::Regex(_, _)
        | Expr::Str(_)
        | Expr::This
        | Expr::Super
        | Expr::NewTarget => false,

        Expr::Template { exprs, .. } => exprs.iter().any(expr_captures),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            expr_captures(tag) || exprs.iter().any(expr_captures)
        }
        Expr::Yield { arg, .. } => arg.as_deref().is_some_and(expr_captures),
        Expr::Await(inner) | Expr::Spread(inner) | Expr::Unary(_, inner) => expr_captures(inner),
        Expr::Array(items) | Expr::Sequence(items) => items.iter().any(expr_captures),
        Expr::Object(props) => props.iter().any(prop_captures),
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => expr_captures(l) || expr_captures(r),
        Expr::Conditional { test, cons, alt } => {
            expr_captures(test) || expr_captures(cons) || expr_captures(alt)
        }
        Expr::Assign { target, value } => expr_captures(target) || expr_captures(value),
        Expr::Update { target, .. } => expr_captures(target),
        Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
            expr_captures(func) || args.iter().any(expr_captures)
        }
        Expr::Member { object, .. } => expr_captures(object),
        Expr::Index { object, index, .. } => expr_captures(object) || expr_captures(index),
    }
}

fn prop_captures(p: &Prop) -> bool {
    match p {
        Prop::KeyValue { key, value, .. } => expr_captures(key) || expr_captures(value),
        Prop::Spread(e) => expr_captures(e),
        // An accessor's `func` is an `Expr::Function`, so this arm is `true`;
        // it is spelled out rather than assumed.
        Prop::Accessor { key, func, .. } => expr_captures(key) || expr_captures(func),
    }
}

/// Does this statement list declare anything that needs a block scope of its
/// own? `let` / `const` / `class` / a hoisted `function` bind into the block;
/// `var` does not (it lands in the enclosing function's base env). A block that
/// binds nothing needs no scope at all, so `{ …; }` inside a hot loop stops
/// allocating and freeing an `EnvData` per pass.
///
/// Direct `eval` forces a scope: `eval("let x = 1")` declares into the running
/// block, and with no block open that binding would escape to the function.
pub fn block_needs_scope(body: &[Stmt]) -> bool {
    body.iter().any(|s| match &s.kind {
        StmtKind::Decl { kind, .. } => !matches!(kind, crate::ast::DeclKind::Var),
        StmtKind::FuncDecl { .. } | StmtKind::ClassDecl(_) => true,
        // Only a DIRECT `eval` in this block's own statements can declare into
        // it; a nested block or function has its own scope to declare into.
        StmtKind::Expr(e) => mentions_eval(e),
        _ => false,
    })
}

/// `eval` named anywhere in this expression (see [`block_needs_scope`]).
fn mentions_eval(e: &Expr) -> bool {
    match e {
        Expr::Ident(n) => n == "eval",
        Expr::Call { func, args, .. } | Expr::New { callee: func, args } => {
            mentions_eval(func) || args.iter().any(mentions_eval)
        }
        Expr::Assign { target, value } => mentions_eval(target) || mentions_eval(value),
        Expr::Sequence(items) => items.iter().any(mentions_eval),
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => mentions_eval(l) || mentions_eval(r),
        Expr::Conditional { test, cons, alt } => {
            mentions_eval(test) || mentions_eval(cons) || mentions_eval(alt)
        }
        Expr::Unary(_, inner) | Expr::Await(inner) | Expr::Spread(inner) => mentions_eval(inner),
        _ => false,
    }
}
