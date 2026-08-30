//! Lower the JavaScript AST to `fusevm::Chunk`.
//!
//! Native fusevm ops carry arithmetic (`+ - * / % **`), the relational
//! comparisons (`< <= > >=`) and boolean short-circuit so the JIT can trace
//! them; the strict numeric hook (host) supplies JS semantics for non-numeric
//! operands (string concat, coercion). Everything JS-specific — name access,
//! member/index access, calls, object/array construction, iteration — lowers to
//! a `CallBuiltin` that lands in `builtins.rs`.
//!
//! Conditions are normalized through the `TRUTHY` builtin before a native
//! `JumpIfFalse`, because JS truthiness differs from fusevm's default numeric
//! truthiness. Compiler-internal name strings travel as native `Value::Str`
//! constants; JS-level strings are always heap objects built by `MKSTR`.

use crate::ast::*;
use crate::host::{binop as bop, member, ops, unop, unwind, FuncDef, ParamSlot, TryDef};
use fusevm::{Chunk, ChunkBuilder, Op, Value};

/// A compiled program: the top-level chunk plus the function template table and
/// the try-block table.
#[derive(Default)]
pub struct Program {
    pub main: Chunk,
    pub functions: Vec<(String, FuncDef)>,
    pub tries: Vec<TryDef>,
}

/// Rebase every func-id and try-id reference so its ids sit above those already
/// loaded on the host (needed only for incremental loading; a no-op for a single
/// run).
pub fn rebase_program(prog: &mut Program, func_off: usize, try_off: usize) {
    if func_off == 0 && try_off == 0 {
        return;
    }
    rebase_chunk(&mut prog.main, func_off, try_off);
    for (_, f) in &mut prog.functions {
        rebase_chunk(&mut f.chunk, func_off, try_off);
    }
    for t in &mut prog.tries {
        rebase_chunk(&mut t.block, func_off, try_off);
        if let Some((_, hb)) = &mut t.handler {
            rebase_chunk(hb, func_off, try_off);
        }
        if let Some(f) = &mut t.finalizer {
            rebase_chunk(f, func_off, try_off);
        }
    }
}

fn rebase_chunk(chunk: &mut Chunk, func_off: usize, try_off: usize) {
    for i in 1..chunk.ops.len() {
        let off = match chunk.ops[i] {
            Op::CallBuiltin(id, _) if id == ops::MKFUNC => func_off,
            Op::CallBuiltin(id, 1) if id == ops::TRY => try_off,
            _ => continue,
        };
        if off == 0 {
            continue;
        }
        if let Op::LoadInt(v) = &mut chunk.ops[i - 1] {
            *v += off as i64;
        }
    }
    for sub in &mut chunk.sub_chunks {
        rebase_chunk(sub, func_off, try_off);
    }
}

/// The binding scope a declaration keyword introduces.
fn bind_mode(kind: DeclKind) -> BindMode {
    match kind {
        DeclKind::Var => BindMode::Var,
        DeclKind::Let => BindMode::Lexical,
        DeclKind::Const => BindMode::Const,
    }
}

/// How a binding site introduces its name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindMode {
    /// Plain assignment to an existing binding (`x = 1`, a for-of head without
    /// `let`/`const`/`var`).
    Assign,
    /// `let`/`class`: bound in the innermost BLOCK scope.
    Lexical,
    /// `const`: block-scoped like `Lexical`, but IMMUTABLE — a later assignment
    /// to the name throws `TypeError: Assignment to constant variable.`
    Const,
    /// `var` / a hoisted function declaration: bound at FUNCTION scope.
    Var,
}

/// Break/continue jump fixups for a loop or switch.
struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    /// Block-scope depth the `break` target expects; a `break` from inside nested
    /// blocks pops back down to it first.
    break_depth: usize,
    /// Block-scope depth the `continue` target expects.
    continue_depth: usize,
    /// Number of iterators on the VM stack inside this loop's body.
    iter_depth: usize,
    /// Whether `continue` binds here (true for loops, false for `switch`).
    catches_continue: bool,
    /// The source label attached to this loop/block, if any (`outer: for …`),
    /// so labeled `break outer` / `continue outer` can target it directly.
    label: Option<String>,
}

#[derive(Default)]
pub struct Compiler {
    /// Pending short-circuit jumps for the optional chain being lowered, one
    /// frame per chain.
    ///
    /// `?.` short-circuits the WHOLE chain to its right, not just its own link:
    /// `o.a?.b.c` is `undefined` when `o.a` is nullish, and never reads `.c`
    /// off it. Each `?.` therefore parks its jump here and the chain's ROOT
    /// patches every one of them to the end. An empty stack means no chain is
    /// open, so a `?.` outside one patches itself as before.
    opt_chain: Vec<Vec<usize>>,
    functions: Vec<(String, FuncDef)>,
    tries: Vec<TryDef>,
    loops: Vec<LoopCtx>,
    tmp: usize,
    /// A label seen immediately before a loop, consumed by that loop's `LoopCtx`
    /// (`outer: for (…)`); `None` once claimed.
    pending_label: Option<String>,
    /// Emit per-statement `DBG_LINE` markers for the DAP debugger (`node --dap`).
    debug: bool,
    /// Index into `loops` of the first loop opened by the chunk being emitted.
    /// A `break`/`continue` targeting a loop BELOW this index leaves the current
    /// chunk (a `try` body is compiled as its own chunk), so it cannot be a plain
    /// jump and is raised as a signal instead.
    chunk_loop_base: usize,
    /// Whether this chunk contains a signal-raising `break`/`continue`, so loops
    /// in it must re-dispatch a still-pending signal when they exit.
    chunk_signals: bool,
    /// Number of block scopes open at the current emission point, so a jump out of
    /// them can pop exactly the right number.
    scope_depth: usize,
    /// True while compiling an `async function*` body, where `yield*` must drive
    /// the delegate through the ASYNC iteration protocol.
    in_async_generator: bool,
    /// Number of for-of/for-in iterators parked on the VM stack at this point. A
    /// `break`/`continue` that leaves such a loop must close and drop its iterator,
    /// otherwise the enclosing loop's `FORITER` would peek at the wrong one.
    iter_depth: usize,
    /// Whether the code being emitted is in STRICT mode — a `'use strict'`
    /// directive prologue on the program or an enclosing function body, or a
    /// class body (which is strict unconditionally). The only difference it
    /// makes here is `PutValue` on an unresolvable reference: strict code throws
    /// `ReferenceError` where sloppy code creates a global.
    strict: bool,
    /// Callee SOURCE TEXT per call op of the chunk being emitted, handed to the
    /// host when the chunk is built so a failed call can name the callee the way
    /// the source wrote it. Saved and restored around every nested chunk.
    call_sites: Vec<(usize, String)>,
    /// Parked-iterator depth per `yield` op of the chunk being emitted, so an
    /// injected `.return()`/`.throw()` can close the `for…of` / `yield*`
    /// iterators the halt would otherwise abandon.
    yield_sites: Vec<(usize, usize)>,
    /// Locals of the chunk being emitted that live in fusevm frame slots rather
    /// than the host's scope chain — see [`crate::slots`]. Empty for a chunk the
    /// analysis refused, so `slot_of` answering `None` is the old path.
    slots: crate::slots::Plan,
}

// ── early errors: duplicate lexical declarations ─────────────────────────────

/// Reject a duplicate lexical declaration before anything runs, as node does.
///
/// `let a = 1; let a = 2;` is a SyntaxError at PARSE time in node, and this
/// engine ran it — the second declaration simply won. That is the gap that lets
/// a genuine double-declaration bug through silently, and it bit three test
/// files in this repo whose collisions node rejected and this accepted.
///
/// Deliberately narrow, since a false positive REJECTS a program that works:
/// only the three collisions the spec is unambiguous about are reported —
/// two lexical declarations of one name in the same statement list, a lexical
/// name that a `var` in the same subtree hoists onto, and a lexical name
/// colliding with a function declaration beside it. Repeated `var`s, and the
/// same name in nested scopes, stay legal.
pub fn check_early_errors(stmts: &[Stmt]) -> Result<(), String> {
    let mut lexical: Vec<String> = Vec::new();
    let mut functions: Vec<String> = Vec::new();
    for st in stmts {
        match &st.kind {
            StmtKind::Decl { kind, decls } if !matches!(kind, DeclKind::Var) => {
                for d in decls {
                    let mut names = Vec::new();
                    pattern_names(&d.target, &mut names);
                    for n in names {
                        if lexical.contains(&n) {
                            return Err(already_declared(&n));
                        }
                        lexical.push(n);
                    }
                }
            }
            StmtKind::ClassDecl(c) => {
                if let Some(n) = &c.name {
                    if lexical.contains(n) {
                        return Err(already_declared(n));
                    }
                    lexical.push(n.clone());
                }
            }
            StmtKind::FuncDecl { name, .. } => functions.push(name.clone()),
            _ => {}
        }
    }
    // A function declaration and a lexical binding of the same name cannot
    // share a scope, whichever order they appear in.
    for f in &functions {
        if lexical.contains(f) {
            return Err(already_declared(f));
        }
    }
    // A `var` anywhere below hoists PAST any block between it and its function
    // scope, so it collides with a lexical name declared here.
    let mut vars: Vec<String> = Vec::new();
    for st in stmts {
        collect_var_names(st, &mut vars);
    }
    for n in &lexical {
        if vars.contains(n) {
            return Err(already_declared(n));
        }
    }
    // Each nested statement list is its own scope.
    for st in stmts {
        check_nested(&st.kind)?;
    }
    Ok(())
}

fn already_declared(name: &str) -> String {
    format!("SyntaxError: Identifier '{name}' has already been declared")
}

/// Recurse into the statement lists that form their own scopes. A function
/// BODY is checked when that function is compiled, so the walk does not
/// descend into one here.
fn check_nested(k: &StmtKind) -> Result<(), String> {
    let one = |s: &Stmt| check_nested(&s.kind);
    match k {
        StmtKind::Block(b) => check_early_errors(b),
        StmtKind::If { cons, alt, .. } => {
            one(cons)?;
            match alt {
                Some(a) => one(a),
                None => Ok(()),
            }
        }
        StmtKind::While { body, .. }
        | StmtKind::DoWhile { body, .. }
        | StmtKind::Labeled { body, .. }
        | StmtKind::ForOf { body, .. }
        | StmtKind::ForIn { body, .. } => one(body),
        StmtKind::For { body, .. } => one(body),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            check_early_errors(block)?;
            if let Some((_, h)) = handler {
                check_early_errors(h)?;
            }
            match finalizer {
                Some(f) => check_early_errors(f),
                None => Ok(()),
            }
        }
        StmtKind::Switch { cases, .. } => {
            // Every case shares ONE block scope, so their statements are
            // checked together rather than case by case.
            let all: Vec<Stmt> = cases.iter().flat_map(|c| c.body.clone()).collect();
            check_early_errors(&all)
        }
        _ => Ok(()),
    }
}

/// Compile a parsed program. `debug` enables per-statement DAP line markers.
pub fn compile(stmts: &[Stmt], debug: bool) -> Result<Program, String> {
    let mut c = Compiler {
        opt_chain: Vec::new(),
        debug,
        // Under `--dap` the debugger reads scopes by name out of the host, and a
        // slot has no name, so a debug run keeps every local a binding.
        slots: if debug {
            Default::default()
        } else {
            crate::slots::plan(&[], stmts, true)
        },
        strict: has_use_strict(stmts),
        ..Default::default()
    };
    check_early_errors(stmts)?;
    let mut b = ChunkBuilder::new();
    // Hoist function declarations to the top (JS function hoisting).
    c.hoist_vars(&mut b, stmts)?;
    c.hoist_funcs(&mut b, stmts)?;
    c.compile_stmts(&mut b, stmts)?;
    Ok(Program {
        main: c.finish_chunk(b),
        functions: c.functions,
        tries: c.tries,
    })
}

/// Compile leaving the value of the final top-level expression statement on the
/// stack (the program's completion value), for `eval`/`vm.runInThisContext`. A
/// non-expression final statement leaves nothing (→ `undefined`).
pub fn compile_completion(stmts: &[Stmt], debug: bool) -> Result<Program, String> {
    let mut c = Compiler {
        opt_chain: Vec::new(),
        debug,
        strict: has_use_strict(stmts),
        ..Default::default()
    };
    let mut b = ChunkBuilder::new();
    c.hoist_vars(&mut b, stmts)?;
    c.hoist_funcs(&mut b, stmts)?;
    if let Some((last, rest)) = stmts.split_last() {
        c.compile_stmts(&mut b, rest)?;
        if let StmtKind::Expr(e) = &last.kind {
            // The final expression's value is NOT popped — it is the completion.
            c.compile_expr(&mut b, e)?;
        } else {
            c.compile_stmt(&mut b, last)?;
        }
    }
    Ok(Program {
        main: c.finish_chunk(b),
        functions: c.functions,
        tries: c.tries,
    })
}

/// Does this statement list open with a `"use strict"` directive prologue?
///
/// A directive prologue is the run of leading statements that are nothing but a
/// string literal, so `"use strict"` counts only while every statement before it
/// is also one.
fn has_use_strict(stmts: &[Stmt]) -> bool {
    for s in stmts {
        match &s.kind {
            StmtKind::Expr(e) => match e {
                Expr::Str(v) if v == "use strict" => return true,
                Expr::Str(_) => continue,
                _ => return false,
            },
            _ => return false,
        }
    }
    false
}

/// The callee's source text, re-printed from its AST the way V8's `CallPrinter`
/// does for the `TypeError` a failed call raises: `o.a.b`, `o[k]`, `"s".x`,
/// `3.x`, `o?.a?.zz`. A string-literal computed access normalizes to dot form
/// (`o['a']` prints `o.a`), which is what node reports.
///
/// `None` for any shape this does not print faithfully — the caller then keeps
/// the bare method name it already used, so an unprinted shape is never given
/// invented text.
fn callee_text(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Ident(n) => n.clone(),
        Expr::This => "this".into(),
        Expr::Number(n) => crate::host::fmt_number(*n),
        Expr::Str(s) => format!("\"{s}\""),
        Expr::True => "true".into(),
        Expr::False => "false".into(),
        Expr::Null => "null".into(),
        Expr::Undefined => "undefined".into(),
        Expr::Array(items) if items.is_empty() => "[]".into(),
        Expr::Object(props) if props.is_empty() => "{}".into(),
        Expr::Member {
            object,
            property,
            optional,
        } => {
            let dot = if *optional { "?." } else { "." };
            format!("{}{dot}{property}", callee_text(object)?)
        }
        Expr::Index {
            object,
            index,
            optional,
        } => {
            let obj = callee_text(object)?;
            // A string-literal key that is a plain identifier prints as a dot
            // access, exactly as node reports it.
            if let Expr::Str(k) = &**index {
                if is_identifier(k) {
                    let dot = if *optional { "?." } else { "." };
                    return Some(format!("{obj}{dot}{k}"));
                }
            }
            let idx = callee_text(index)?;
            let open = if *optional { "?.[" } else { "[" };
            format!("{obj}{open}{idx}]")
        }
        // V8 prints a call in a callee position as `f(...)`, whatever its
        // arguments were: `require('fs').nope()` reports `require(...).nope`.
        Expr::Call { func, .. } => format!("{}(...)", callee_text(func)?),
        Expr::Sequence(items) => {
            let parts: Option<Vec<String>> = items.iter().map(callee_text).collect();
            format!("({})", parts?.join(" , "))
        }
        _ => return None,
    })
}

/// Whether `s` can be written after a `.` — the test that decides whether a
/// string-literal computed access prints in dot form.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn argc(n: usize) -> Result<u8, String> {
    u8::try_from(n).map_err(|_| "too many arguments (>255) for one call".to_string())
}

/// Does this expression already leave a `Value::Bool` on the stack? A condition
/// that does needs no `TRUTHY` call: `JumpIfFalse` reads the boolean directly.
///
/// The gain is one host round-trip per condition evaluation — `for (let i = 0;
/// i < n; i++)` paid it on every iteration — and it also puts the comparison
/// immediately before the jump that consumes it, which is what fusevm's block
/// JIT requires of a bool-producing op (`bool_is_consumed_in_place`).
///
/// Every arm listed here is a lowering that ends in a `Bool`: the relational
/// ops go to `Op::Num{Lt,Le,Gt,Ge}` (the numeric hook's `relational` returns a
/// Rust `bool`), the equality ops to `STRICT_EQ`/`LOOSE_EQ`, `in` to
/// `CONTAINS`, `instanceof` to `INSTANCEOF`, and `!`/`!=`/`!==` end in
/// `Op::LogNot`. Anything else — including `&&`/`||`/`??`, which evaluate to an
/// OPERAND and not to a boolean — keeps the call.
fn yields_bool(e: &Expr) -> bool {
    match e {
        Expr::True | Expr::False => true,
        Expr::Unary(UnOp::Not, _) | Expr::Unary(UnOp::Delete, _) => true,
        Expr::Binary(op, _, _) => matches!(
            op,
            BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::EqEq
                | BinOp::NeEq
                | BinOp::EqEqEq
                | BinOp::NeEqEq
                | BinOp::In
                | BinOp::InstanceOf
        ),
        _ => false,
    }
}

impl Compiler {
    // ── emit helpers ─────────────────────────────────────────────────────
    fn name_const(&self, b: &mut ChunkBuilder, s: &str) {
        let k = b.add_constant(Value::str(s));
        b.emit(Op::LoadConst(k), 0);
    }
    fn strlit(&self, b: &mut ChunkBuilder, s: &str) {
        let k = b.add_constant(Value::str(s));
        b.emit(Op::LoadConst(k), 0);
        b.emit(Op::CallBuiltin(ops::MKSTR, 1), 0);
    }
    fn tmp_name(&mut self, tag: &str) -> String {
        let n = format!(".{tag}{}", self.tmp);
        self.tmp += 1;
        n
    }

    /// Emit MKFUNC for a compiled function template and leave the closure on the
    /// stack.
    fn emit_mkfunc(&self, b: &mut ChunkBuilder, def_id: usize) {
        b.emit(Op::LoadInt(def_id as i64), 0);
        b.emit(Op::CallBuiltin(ops::MKFUNC, 1), 0);
    }

    /// Emit the `var` hoisting for one function (or program) scope.
    ///
    /// A `var` binding exists from the moment its scope is entered, so
    /// `f(){ x; var x = 1 }` reads `undefined` where a `let` would throw. The
    /// walk therefore descends through every block, loop, `switch`, `try` and
    /// label — `var` ignores block scope — but stops at a nested function, which
    /// begins a scope of its own. Only the binding is created here; the
    /// initialiser still runs where it is written.
    ///
    /// Emitted BEFORE [`Self::hoist_funcs`] so a function declaration overwrites
    /// the `undefined` rather than the other way round, which is the order the
    /// spec instantiates them in.
    fn hoist_vars(&mut self, b: &mut ChunkBuilder, stmts: &[Stmt]) -> Result<(), String> {
        let mut names = Vec::new();
        for s in stmts {
            collect_var_names(s, &mut names);
        }
        for n in names {
            // A slotted local is its slot, which already reads `undefined`
            // before its first write, so there is no binding to create.
            if self.slot_of(&n).is_some() {
                continue;
            }
            self.name_const(b, &n);
            b.emit(Op::CallBuiltin(ops::HOIST_VAR, 1), 0);
            b.emit(Op::Pop, 0);
        }
        Ok(())
    }

    fn hoist_funcs(&mut self, b: &mut ChunkBuilder, stmts: &[Stmt]) -> Result<(), String> {
        for s in stmts {
            if let StmtKind::FuncDecl {
                name,
                params,
                body,
                is_generator,
                is_async,
            } = &s.kind
            {
                let def_id = self.build_function(name, params, body, *is_generator, *is_async)?;
                self.emit_mkfunc(b, def_id);
                // Function declarations hoist to the enclosing FUNCTION scope.
                self.declare_as(b, &Expr::Ident(name.clone()), BindMode::Var);
            }
        }
        Ok(())
    }

    fn compile_stmts(&mut self, b: &mut ChunkBuilder, stmts: &[Stmt]) -> Result<(), String> {
        for s in stmts {
            self.compile_stmt(b, s)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, b: &mut ChunkBuilder, s: &Stmt) -> Result<(), String> {
        if self.debug && s.line != 0 {
            b.emit(Op::LoadInt(s.line as i64), s.line);
            b.emit(Op::CallBuiltin(ops::DBG_LINE, 1), s.line);
            b.emit(Op::Pop, s.line);
        }
        let line = s.line;
        match &s.kind {
            StmtKind::Expr(e) => {
                self.compile_expr(b, e)?;
                b.emit(Op::Pop, line);
            }
            StmtKind::Empty => {}
            StmtKind::FuncDecl { .. } => {} // hoisted at block entry
            StmtKind::ClassDecl(node) => {
                self.compile_class(b, node)?;
                // Bind the class to its name in the current scope.
                if let Some(name) = &node.name {
                    self.declare(b, &Expr::Ident(name.clone()));
                } else {
                    b.emit(Op::Pop, line);
                }
            }
            StmtKind::Decl { kind, decls } => {
                let mode = bind_mode(*kind);
                for d in decls {
                    // `var x;` with no initialiser names a binding that scope
                    // entry already created, and must NOT reset it — in
                    // `function f(a) { var a; }` the parameter stands.
                    if d.init.is_none() && *kind == DeclKind::Var {
                        continue;
                    }
                    match &d.init {
                        Some(v) => {
                            self.compile_expr(b, v)?;
                            // Name inference: `const f = () => {}` / `= function(){}`
                            // / `= class {}` gives the function/class the name `f`.
                            if let Expr::Ident(name) = &d.target {
                                self.infer_name(b, v, name);
                            }
                        }
                        None => {
                            b.emit(Op::LoadUndef, line);
                        }
                    }
                    self.compile_bind(b, &d.target, mode)?;
                }
            }
            StmtKind::Block(body) => {
                // A block that declares nothing lexical has nothing to put in a
                // scope, and opening one costs an `EnvData` allocation and free
                // every time control enters the block — once per iteration when
                // the block is a loop body, which is where most of them are.
                let scoped = crate::capture::block_needs_scope(body);
                if scoped {
                    self.emit_push_scope(b);
                }
                self.hoist_funcs(b, body)?;
                self.compile_stmts(b, body)?;
                if scoped {
                    self.emit_pop_scope(b);
                }
            }
            StmtKind::If { test, cons, alt } => self.compile_if(b, test, cons, alt)?,
            StmtKind::While { test, body } => self.compile_while(b, test, body)?,
            StmtKind::DoWhile { body, test } => self.compile_do_while(b, body, test)?,
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => self.compile_for(b, init, test, update, body)?,
            StmtKind::ForOf {
                decl_kind,
                target,
                iter,
                body,
                is_await,
            } => {
                let mode = decl_kind.map(bind_mode).unwrap_or(BindMode::Assign);
                if *is_await {
                    self.compile_for_await(b, mode, target, iter, body)?
                } else {
                    self.compile_for_of(b, mode, target, iter, body)?
                }
            }
            StmtKind::ForIn {
                decl_kind,
                target,
                object,
                body,
            } => {
                let mode = decl_kind.map(bind_mode).unwrap_or(BindMode::Assign);
                self.compile_for_in(b, mode, target, object, body)?
            }
            StmtKind::Switch { disc, cases } => self.compile_switch(b, disc, cases)?,
            StmtKind::Return(e) => {
                match e {
                    Some(e) => self.compile_expr(b, e)?,
                    None => {
                        b.emit(Op::LoadUndef, line);
                    }
                }
                // A `return` out of a `for…of` is an abrupt completion, and
                // 7.4.9 `IteratorClose` runs the iterator's `return` for it —
                // which is what makes a generator's `finally` run. `break` and
                // `continue` already closed theirs; a `return` walked away and
                // left the iterator suspended forever.
                self.emit_close_iters_under_value(b);
                b.emit(Op::CallBuiltin(ops::SIG_RETURN, 1), line);
            }
            StmtKind::Labeled { label, body } => self.compile_labeled(b, label, body)?,
            StmtKind::Break(label) => {
                let idx = match label {
                    // `break outer`: the nearest enclosing context carrying that label.
                    Some(name) => self
                        .loops
                        .iter()
                        .rposition(|c| c.label.as_deref() == Some(name.as_str()))
                        .ok_or_else(|| format!("SyntaxError: Undefined label '{name}'"))?,
                    None => self
                        .loops
                        .len()
                        .checked_sub(1)
                        .ok_or("SyntaxError: 'break' outside loop")?,
                };
                if idx >= self.chunk_loop_base {
                    self.emit_unwind_scopes(b, self.loops[idx].break_depth);
                    self.emit_close_iters(b, self.loops[idx].iter_depth);
                    let j = b.emit(Op::Jump(0), line);
                    self.loops[idx].breaks.push(j);
                } else {
                    self.emit_signal_jump(b, ops::SIG_BREAK, label.as_deref(), line);
                }
            }
            StmtKind::Continue(label) => {
                let idx = match label {
                    // `continue outer`: the labeled loop (a label on a non-loop
                    // cannot catch `continue`).
                    Some(name) => self
                        .loops
                        .iter()
                        .rposition(|c| {
                            c.catches_continue && c.label.as_deref() == Some(name.as_str())
                        })
                        .ok_or_else(|| {
                            format!("SyntaxError: Undefined label '{name}' for continue")
                        })?,
                    None => self
                        .loops
                        .iter()
                        .rposition(|c| c.catches_continue)
                        .ok_or("SyntaxError: 'continue' outside loop")?,
                };
                if idx >= self.chunk_loop_base {
                    self.emit_unwind_scopes(b, self.loops[idx].continue_depth);
                    self.emit_close_iters(b, self.loops[idx].iter_depth);
                    let j = b.emit(Op::Jump(0), line);
                    self.loops[idx].continues.push(j);
                } else {
                    self.emit_signal_jump(b, ops::SIG_CONTINUE, label.as_deref(), line);
                }
            }
            StmtKind::Throw(e) => {
                self.compile_expr(b, e)?;
                b.emit(Op::CallBuiltin(ops::THROW, 1), line);
            }
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => self.compile_try(b, block, handler, finalizer)?,
        }
        Ok(())
    }

    // ── binding / assignment ─────────────────────────────────────────────
    /// Store the value on top of the stack into `target`. `declare` chooses
    /// `DECLARE` (new binding) vs `SETLOCAL` (existing binding / global).
    fn compile_bind(
        &mut self,
        b: &mut ChunkBuilder,
        target: &Expr,
        declare: BindMode,
    ) -> Result<(), String> {
        match target {
            Expr::Ident(_) => {
                if declare == BindMode::Assign {
                    self.store_simple(b, target)?;
                } else {
                    self.declare_as(b, target, declare);
                }
            }
            Expr::Member { .. } | Expr::Index { .. } => {
                self.store_simple(b, target)?;
            }
            Expr::Array(items) => self.destructure_array(b, items, declare)?,
            Expr::Object(props) => self.destructure_object(b, props, declare)?,
            Expr::Assign { target, value } => {
                // Pattern element with a default: use it when TOS is undefined.
                b.emit(Op::Dup, 0);
                b.emit(Op::LoadUndef, 0);
                b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
                let jf = b.emit(Op::JumpIfFalse(0), 0);
                b.emit(Op::Pop, 0); // drop the undefined
                self.compile_expr(b, value)?;
                // 8.6.3 / 14.3.3: a destructuring default whose target is a
                // single binding identifier names an anonymous function after
                // it — `const {a = function(){}} = {}` gives `a.name === "a"`.
                if let Expr::Ident(n) = &**target {
                    self.infer_name(b, value, n);
                }
                let end = b.current_pos();
                b.patch_jump(jf, end);
                self.compile_bind(b, target, declare)?;
            }
            _ => return Err("SyntaxError: invalid assignment target".into()),
        }
        Ok(())
    }

    /// Emit a `DECLARE` of a simple name binding, consuming TOS value.
    fn declare(&self, b: &mut ChunkBuilder, target: &Expr) {
        self.declare_as(b, target, BindMode::Lexical);
    }

    /// Emit the declaration op matching `mode`: block-scoped for `let`/`const`,
    /// function-scoped for `var` and hoisted function declarations.
    fn declare_as(&self, b: &mut ChunkBuilder, target: &Expr, mode: BindMode) {
        if let Expr::Ident(n) = target {
            // A slotted local has no scope entry to declare into: the binding IS
            // the store.
            if let Some(slot) = self.slot_of(n) {
                b.emit(Op::SetSlot(slot), 0);
                return;
            }
            let op = match mode {
                BindMode::Var => ops::DECLARE_VAR,
                BindMode::Const => ops::DECLARE_CONST,
                _ => ops::DECLARE,
            };
            self.name_const(b, n);
            b.emit(Op::Swap, 0);
            b.emit(Op::CallBuiltin(op, 2), 0);
            b.emit(Op::Pop, 0);
        }
    }

    /// Emit `throw new TypeError("Assignment to constant variable.")`.
    ///
    /// A store to a `const` is a RUNTIME error, not a parse error — the spec
    /// puts it in SetMutableBinding (8.5.2), so `try { const c=1; c=2 } catch {}`
    /// has to catch it. Emitting the throw in place of the store gives exactly
    /// that, and costs nothing for every store that is not to a const.
    fn throw_const_assignment(&mut self, b: &mut ChunkBuilder) {
        let e = Expr::New {
            callee: Box::new(Expr::Ident("TypeError".into())),
            args: vec![Expr::Str("Assignment to constant variable.".into())],
        };
        // `New` of a known builtin with a literal argument cannot fail to
        // compile, so the error path is unreachable rather than swallowed.
        if self.compile_expr(b, &e).is_ok() {
            b.emit(Op::CallBuiltin(ops::THROW, 1), 0);
        }
    }

    /// Store TOS into an lvalue (Ident/Member/Index), leaving nothing.
    fn store_simple(&mut self, b: &mut ChunkBuilder, target: &Expr) -> Result<(), String> {
        match target {
            Expr::Ident(n) => {
                // A slotted binding never reaches the host's scope chain, so the
                // host's immutable-binding check cannot see it. The slot plan is
                // exact about which names are const (one declaration per name,
                // unreachable from another chunk, simple identifiers only), so
                // the store is rejected here instead — at run time, as the spec
                // requires, since `try { const c=1; c=2 } catch {}` must CATCH
                // this rather than fail to parse.
                if self.slots.consts.contains(n) {
                    b.emit(Op::Pop, 0); // drop the value that will never be stored
                    self.throw_const_assignment(b);
                    return Ok(());
                }
                if let Some(slot) = self.slot_of(n) {
                    b.emit(Op::SetSlot(slot), 0);
                    return Ok(());
                }
                self.name_const(b, n);
                b.emit(Op::Swap, 0);
                // `PutValue` (6.2.5.6) on an unresolvable reference: strict code
                // throws `ReferenceError`, sloppy code creates a global.
                let op = if self.strict {
                    ops::SETLOCAL_STRICT
                } else {
                    ops::SETLOCAL
                };
                b.emit(Op::CallBuiltin(op, 2), 0);
                b.emit(Op::Pop, 0);
            }
            Expr::Member {
                object, property, ..
            } => {
                self.compile_expr(b, object)?; // [value, recv]
                self.name_const(b, property); // [value, recv, name]
                b.emit(Op::Rot, 0); // [recv, name, value]
                b.emit(Op::CallBuiltin(ops::SETATTR, 3), 0);
                b.emit(Op::Pop, 0);
            }
            Expr::Index { object, index, .. } => {
                self.compile_expr(b, object)?; // [value, recv]
                self.compile_expr(b, index)?; // [value, recv, idx]
                b.emit(Op::Rot, 0); // [recv, idx, value]
                b.emit(Op::CallBuiltin(ops::SETITEM, 3), 0);
                b.emit(Op::Pop, 0);
            }
            _ => return Err("SyntaxError: invalid assignment target".into()),
        }
        Ok(())
    }

    fn destructure_array(
        &mut self,
        b: &mut ChunkBuilder,
        items: &[Expr],
        declare: BindMode,
    ) -> Result<(), String> {
        let star_idx = items
            .iter()
            .position(|e| matches!(e, Expr::Spread(_)))
            .map(|i| i as i64)
            .unwrap_or(-1);
        b.emit(Op::LoadInt(items.len() as i64), 0);
        b.emit(Op::LoadInt(star_idx), 0);
        b.emit(Op::CallBuiltin(ops::UNPACK, 3), 0); // pushes items[0]..items[n-1], items[0] on top
        for it in items {
            match it {
                // An elided target position (`const [a, , b] = xs`) still
                // consumes its unpacked value; nothing is bound to it.
                Expr::Hole | Expr::Undefined => {
                    b.emit(Op::Pop, 0);
                }
                Expr::Spread(inner) => self.compile_bind(b, inner, declare)?,
                _ => self.compile_bind(b, it, declare)?,
            }
        }
        Ok(())
    }

    fn destructure_object(
        &mut self,
        b: &mut ChunkBuilder,
        props: &[Prop],
        declare: BindMode,
    ) -> Result<(), String> {
        // Object value on TOS; keep it, read each key, bind, then drop.
        let obj_tmp = self.tmp_name("destr");
        self.name_const(b, &obj_tmp);
        b.emit(Op::Swap, 0);
        b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
        b.emit(Op::Pop, 0);
        // Collect statically-known destructured key names, for a `...rest`.
        let mut named: Vec<String> = Vec::new();
        for p in props {
            match p {
                Prop::KeyValue { key, value, .. } => {
                    if let Expr::Str(s) = key {
                        named.push(s.clone());
                    }
                    // Load obj, read key.
                    self.load_local(b, &obj_tmp);
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0); // [value]
                    self.compile_bind(b, value, declare)?;
                }
                Prop::Spread(target) => {
                    self.load_local(b, &obj_tmp);
                    for k in &named {
                        self.strlit(b, k);
                    }
                    b.emit(Op::CallBuiltin(ops::MKARR, argc(named.len())?), 0);
                    b.emit(Op::CallBuiltin(ops::OBJ_REST, 2), 0); // [rest_object]
                    self.compile_bind(b, target, declare)?;
                }
                // Accessors never appear in a destructuring pattern.
                Prop::Accessor { .. } => {}
            }
        }
        Ok(())
    }

    fn load_local(&self, b: &mut ChunkBuilder, name: &str) {
        if let Some(slot) = self.slot_of(name) {
            b.emit(Op::GetSlot(slot), 0);
            return;
        }
        self.name_const(b, name);
        b.emit(Op::CallBuiltin(ops::GETLOCAL, 1), 0);
    }

    /// The frame slot holding `name` in the chunk being emitted, if it has one.
    fn slot_of(&self, name: &str) -> Option<u16> {
        self.slots.table.get(name).copied()
    }

    /// The slot for `name` if it also provably holds a Number, so `++`/`--` can
    /// be a native add rather than a `NUM_STEP` round-trip through the host.
    fn numeric_slot_of(&self, name: &str) -> Option<u16> {
        self.slots
            .numeric
            .contains(name)
            .then(|| self.slot_of(name))
            .flatten()
    }

    // ── control flow ─────────────────────────────────────────────────────
    fn compile_condition(&mut self, b: &mut ChunkBuilder, e: &Expr) -> Result<(), String> {
        self.compile_expr(b, e)?;
        if !yields_bool(e) {
            b.emit(Op::CallBuiltin(ops::TRUTHY, 1), 0);
        }
        Ok(())
    }

    fn compile_if(
        &mut self,
        b: &mut ChunkBuilder,
        test: &Expr,
        cons: &Stmt,
        alt: &Option<Box<Stmt>>,
    ) -> Result<(), String> {
        self.compile_condition(b, test)?;
        let jfalse = b.emit(Op::JumpIfFalse(0), 0);
        self.compile_stmt(b, cons)?;
        if let Some(alt) = alt {
            let jend = b.emit(Op::Jump(0), 0);
            let else_start = b.current_pos();
            b.patch_jump(jfalse, else_start);
            self.compile_stmt(b, alt)?;
            let end = b.current_pos();
            b.patch_jump(jend, end);
        } else {
            let end = b.current_pos();
            b.patch_jump(jfalse, end);
        }
        Ok(())
    }

    /// `label: stmt`. If the body is a loop, the label rides into that loop's
    /// `LoopCtx` (so labeled `break`/`continue` target it); otherwise a break-only
    /// context spans the body so `break label` can jump past it.
    fn compile_labeled(
        &mut self,
        b: &mut ChunkBuilder,
        label: &str,
        body: &Stmt,
    ) -> Result<(), String> {
        if matches!(
            body.kind,
            StmtKind::While { .. }
                | StmtKind::DoWhile { .. }
                | StmtKind::For { .. }
                | StmtKind::ForOf { .. }
                | StmtKind::ForIn { .. }
        ) {
            self.pending_label = Some(label.to_string());
            self.compile_stmt(b, body)?;
            // The loop claimed it; clear any residue defensively.
            self.pending_label = None;
        } else {
            self.loops.push(LoopCtx {
                breaks: Vec::new(),
                continues: Vec::new(),
                break_depth: self.scope_depth,
                continue_depth: self.scope_depth,
                iter_depth: self.iter_depth,
                catches_continue: false,
                label: Some(label.to_string()),
            });
            self.compile_stmt(b, body)?;
            let ctx = self.loops.pop().unwrap();
            let end = b.current_pos();
            for br in ctx.breaks {
                b.patch_jump(br, end);
            }
            self.redispatch_after_loop(b);
        }
        Ok(())
    }

    /// After a loop/switch exits, a signal raised deeper in this chunk may still be
    /// pending (a LABELED `break`/`continue` for an OUTER loop). Re-dispatch it one
    /// level out. Emitted only when this chunk actually raises signals.
    fn redispatch_after_loop(&mut self, b: &mut ChunkBuilder) {
        if self.chunk_signals {
            self.emit_signal_dispatch(b);
        }
    }

    /// `while (test) body`, lowered ROTATED: the test is emitted once as an entry
    /// guard and once at the bottom, so the loop closes with a CONDITIONAL
    /// backward branch rather than an unconditional `Jump` back to a test at the
    /// top.
    ///
    /// That shape is what fusevm's tracing JIT needs — it only closes a trace on
    /// a conditional backward branch. Emitted the other way, `--tiers` reported
    /// `trace-eligible=true traced=false` and `reaches native code false` for
    /// every `for` and `while` this frontend produced, while the same arithmetic
    /// written as `do { … } while (…)` — the one loop form that already ended in
    /// a conditional branch — reported `traced=true`. Measured on a debug build:
    /// `for (let i = 0; i < 3000000; i++) s += i` took 5.76s of user CPU
    /// unrotated and 0.02s rotated.
    ///
    /// Evaluation order and count are unchanged: a top-test loop runs the test
    /// `n + 1` times for `n` iterations, and so does this — one entry test, then
    /// one after each pass. Rotation costs one copy of the condition's code and
    /// saves one jump per iteration.
    fn compile_while(
        &mut self,
        b: &mut ChunkBuilder,
        test: &Expr,
        body: &Stmt,
    ) -> Result<(), String> {
        self.compile_condition(b, test)?;
        let jfalse = b.emit(Op::JumpIfFalse(0), 0);
        let top = b.current_pos();
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: true,
            label: self.pending_label.take(),
        });
        self.compile_stmt(b, body)?;
        // `continue` re-tests the condition, which is now the BOTTOM copy of it.
        let cont_target = b.current_pos();
        self.compile_condition(b, test)?;
        b.emit(Op::JumpIfTrue(top), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, cont_target);
        }
        let end = b.current_pos();
        b.patch_jump(jfalse, end);
        for br in ctx.breaks {
            b.patch_jump(br, end);
        }
        self.redispatch_after_loop(b);
        Ok(())
    }

    fn compile_do_while(
        &mut self,
        b: &mut ChunkBuilder,
        body: &Stmt,
        test: &Expr,
    ) -> Result<(), String> {
        let start = b.current_pos();
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: true,
            label: self.pending_label.take(),
        });
        self.compile_stmt(b, body)?;
        let cont_target = b.current_pos();
        self.compile_condition(b, test)?;
        b.emit(Op::JumpIfTrue(start), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, cont_target);
        }
        let end = b.current_pos();
        for br in ctx.breaks {
            b.patch_jump(br, end);
        }
        self.redispatch_after_loop(b);
        Ok(())
    }

    fn compile_for(
        &mut self,
        b: &mut ChunkBuilder,
        init: &Option<Box<Stmt>>,
        test: &Option<Expr>,
        update: &Option<Expr>,
        body: &Stmt,
    ) -> Result<(), String> {
        // A `let`/`const` head is scoped to the loop AND re-bound per iteration, so
        // a closure made in one pass keeps that pass's value (ForBodyEvaluation's
        // CreatePerIterationEnvironment). A `var` head belongs to the function.
        let lexical_head = matches!(
            init.as_deref(),
            Some(Stmt {
                kind: StmtKind::Decl {
                    kind: DeclKind::Let | DeclKind::Const,
                    ..
                },
                ..
            })
        );
        // The loop's own scope is not optional — it is what keeps `let i` from
        // leaking past the loop or clobbering an outer `i`. The per-iteration
        // COPY of that scope is: only code that can CAPTURE a binding can tell
        // one copy per pass from one binding mutated in place, and the copy is a
        // whole-scope clone every iteration. A 5M-iteration counting loop spent
        // 17% of its samples cloning scopes that nothing could observe.
        let per_iteration = lexical_head;
        let copy_per_iteration = lexical_head
            && (crate::capture::stmt_captures(body)
                || init.as_deref().is_some_and(crate::capture::stmt_captures)
                || test.as_ref().is_some_and(crate::capture::expr_captures)
                || update.as_ref().is_some_and(crate::capture::expr_captures));
        if per_iteration {
            self.emit_push_scope(b);
        }
        if let Some(init) = init {
            self.compile_stmt(b, init)?;
        }
        if copy_per_iteration {
            self.emit_copy_scope(b);
        }
        // Rotated, for the reason `compile_while` documents: the test as an entry
        // guard plus a conditional backward branch at the bottom.
        let jfalse = match test {
            Some(t) => {
                self.compile_condition(b, t)?;
                Some(b.emit(Op::JumpIfFalse(0), 0))
            }
            None => None,
        };
        let top = b.current_pos();
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: true,
            label: self.pending_label.take(),
        });
        self.compile_stmt(b, body)?;
        let cont_target = b.current_pos();
        if copy_per_iteration {
            // Fresh copy BEFORE the update, so the update advances the NEXT pass's
            // binding and the one just captured keeps this pass's value.
            self.emit_copy_scope(b);
        }
        if let Some(u) = update {
            self.compile_expr(b, u)?;
            b.emit(Op::Pop, 0);
        }
        match test {
            Some(t) => {
                self.compile_condition(b, t)?;
                b.emit(Op::JumpIfTrue(top), 0);
            }
            // `for (;;)` has no test to branch on, so the back edge is a
            // constant-true CONDITIONAL branch rather than an unconditional
            // `Jump`. The distinction is not cosmetic: fusevm's trace compiler
            // only ever installs a trace closed by `JumpIfTrue`/`JumpIfFalse`
            // and silently declines an `Op::Jump` close, so `for (;;)` stayed
            // interpreted while the identical `while (true)` — which already
            // emitted `LoadTrue; JumpIfTrue` — reached native code. Measured on
            // a debug build, 3M iterations of `s += i`: 4.26s against 0.02s.
            None => {
                b.emit(Op::LoadTrue, 0);
                b.emit(Op::JumpIfTrue(top), 0);
            }
        }
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, cont_target);
        }
        let end = b.current_pos();
        if let Some(jf) = jfalse {
            b.patch_jump(jf, end);
        }
        for br in ctx.breaks {
            b.patch_jump(br, end);
        }
        if per_iteration {
            self.emit_pop_scope(b);
        }
        self.redispatch_after_loop(b);
        Ok(())
    }

    fn compile_for_of(
        &mut self,
        b: &mut ChunkBuilder,
        declare: BindMode,
        target: &Expr,
        iter: &Expr,
        body: &Stmt,
    ) -> Result<(), String> {
        self.compile_expr(b, iter)?;
        b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
        self.iter_depth += 1;
        let r = self.loop_over(b, declare, target, body);
        self.iter_depth -= 1;
        r
    }

    fn compile_for_in(
        &mut self,
        b: &mut ChunkBuilder,
        declare: BindMode,
        target: &Expr,
        object: &Expr,
        body: &Stmt,
    ) -> Result<(), String> {
        self.compile_expr(b, object)?;
        b.emit(Op::CallBuiltin(ops::FORIN_KEYS, 1), 0); // [keys_array]
        b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
        self.iter_depth += 1;
        let r = self.loop_over(b, declare, target, body);
        self.iter_depth -= 1;
        r
    }

    /// `for await (target of iterable) body`. Obtains an async iterator, then each
    /// pass `await`s a `{value, done}` step (a native async iterator's promise, or
    /// the sync fallback's per-value await). The iterator lives in a temp local.
    fn compile_for_await(
        &mut self,
        b: &mut ChunkBuilder,
        declare: BindMode,
        target: &Expr,
        iter: &Expr,
        body: &Stmt,
    ) -> Result<(), String> {
        let iter_tmp = self.tmp_name("aiter");
        self.compile_expr(b, iter)?;
        b.emit(Op::CallBuiltin(ops::GET_ASYNC_ITER, 1), 0); // [iterator]
        self.name_const(b, &iter_tmp);
        b.emit(Op::Swap, 0);
        b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
        b.emit(Op::Pop, 0);
        let start = b.current_pos();
        // step = await ASYNC_STEP(iterator)  -> {value, done}
        self.load_local(b, &iter_tmp);
        b.emit(Op::CallBuiltin(ops::ASYNC_STEP, 1), 0); // [stepPromise]
        b.emit(Op::CallBuiltin(ops::AWAIT, 1), 0); // [step]
        let step_tmp = self.tmp_name("astep");
        self.name_const(b, &step_tmp);
        b.emit(Op::Swap, 0);
        b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
        b.emit(Op::Pop, 0);
        // if (step.done) break
        self.load_local(b, &step_tmp);
        self.name_const(b, "done");
        b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
        b.emit(Op::CallBuiltin(ops::TRUTHY, 1), 0);
        let jdone = b.emit(Op::JumpIfTrue(0), 0);
        // target = step.value
        self.load_local(b, &step_tmp);
        self.name_const(b, "value");
        b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [value]
        let per_iteration = matches!(declare, BindMode::Lexical | BindMode::Const);
        if per_iteration {
            self.emit_push_scope(b);
        }
        self.compile_bind(b, target, declare)?;
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: true,
            label: self.pending_label.take(),
        });
        self.compile_stmt(b, body)?;
        let cont_target = b.current_pos();
        if per_iteration {
            self.emit_pop_scope(b);
        }
        b.emit(Op::Jump(start), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, cont_target);
        }
        // `done` arrives before the iteration scope is open; `break` from inside it
        // still has one to close.
        let break_target = b.current_pos();
        if per_iteration {
            b.emit(Op::CallBuiltin(ops::POP_SCOPE, 0), 0);
            b.emit(Op::Pop, 0);
        }
        // Leaving early closes the async iterator, running an async generator's
        // pending `finally` / calling a user iterator's `.return()`.
        self.load_local(b, &iter_tmp);
        b.emit(Op::CallBuiltin(ops::ITER_CLOSE, 1), 0);
        b.emit(Op::Pop, 0);
        let end = b.current_pos();
        b.patch_jump(jdone, end);
        for br in ctx.breaks {
            b.patch_jump(br, break_target);
        }
        self.redispatch_after_loop(b);
        Ok(())
    }

    /// Shared loop tail for for-of / for-in: iterator on TOS.
    fn loop_over(
        &mut self,
        b: &mut ChunkBuilder,
        declare: BindMode,
        target: &Expr,
        body: &Stmt,
    ) -> Result<(), String> {
        // `for (const v of …)` binds a FRESH `v` each pass, so a closure made in one
        // pass keeps that pass's element.
        let per_iteration = matches!(declare, BindMode::Lexical | BindMode::Const);
        let start = b.current_pos();
        b.emit(Op::CallBuiltin(ops::FORITER, 0), 0); // [iterator, value, has_next]
        let jdone = b.emit(Op::JumpIfFalse(0), 0); // pops has_next
        if per_iteration {
            self.emit_push_scope(b);
        }
        self.compile_bind(b, target, declare)?; // consumes value -> [iterator]
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: true,
            label: self.pending_label.take(),
        });
        self.compile_stmt(b, body)?;
        let cont_target = b.current_pos();
        if per_iteration {
            self.emit_pop_scope(b);
        }
        b.emit(Op::Jump(start), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, cont_target);
        }
        let done = b.current_pos();
        b.patch_jump(jdone, done);
        b.emit(Op::Pop, 0); // drop iterator
        let jafter = b.emit(Op::Jump(0), 0);
        let break_target = b.current_pos();
        // `break` out of a for-of closes the iterator (runs a generator's pending
        // `finally` / calls a user iterator's `.return()`), then drops it.
        if per_iteration {
            b.emit(Op::CallBuiltin(ops::POP_SCOPE, 0), 0);
            b.emit(Op::Pop, 0);
        }
        b.emit(Op::CallBuiltin(ops::ITER_CLOSE, 1), 0);
        b.emit(Op::Pop, 0); // ITER_CLOSE leaves its result; the `done` path popped
        let end = b.current_pos();
        b.patch_jump(jafter, end);
        for br in ctx.breaks {
            b.patch_jump(br, break_target);
        }
        // Every exit path above has already closed and dropped THIS loop's
        // iterator, so a signal re-dispatched here must not count it as live.
        self.iter_depth -= 1;
        self.redispatch_after_loop(b);
        self.iter_depth += 1;
        Ok(())
    }

    fn compile_switch(
        &mut self,
        b: &mut ChunkBuilder,
        disc: &Expr,
        cases: &[SwitchCase],
    ) -> Result<(), String> {
        let disc_tmp = self.tmp_name("switch");
        self.compile_expr(b, disc)?;
        self.name_const(b, &disc_tmp);
        b.emit(Op::Swap, 0);
        b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
        b.emit(Op::Pop, 0);
        // All cases share ONE block scope, so `case 1: let x = …` is visible to the
        // later cases but dies with the switch. It opens BEFORE the test chain
        // because each case test jumps straight into its body.
        self.emit_push_scope(b);
        // Emit the test chain: `if (disc === caseTest) goto bodyN`.
        let mut body_jumps: Vec<Option<usize>> = Vec::new();
        let mut default_idx: Option<usize> = None;
        for (i, case) in cases.iter().enumerate() {
            match &case.test {
                Some(t) => {
                    self.load_local(b, &disc_tmp);
                    self.compile_expr(b, t)?;
                    b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
                    let j = b.emit(Op::JumpIfTrue(0), 0);
                    body_jumps.push(Some(j));
                }
                None => {
                    default_idx = Some(i);
                    body_jumps.push(None);
                }
            }
        }
        // No test matched: jump to default (if any) or end.
        let no_match_jump = b.emit(Op::Jump(0), 0);
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.scope_depth,
            continue_depth: self.scope_depth,
            iter_depth: self.iter_depth,
            catches_continue: false,
            label: None,
        });
        let mut body_starts: Vec<usize> = Vec::new();
        for case in cases {
            body_starts.push(b.current_pos());
            self.compile_stmts(b, &case.body)?;
        }
        let end = b.current_pos();
        // Patch each case test-jump to its body start.
        for (i, j) in body_jumps.iter().enumerate() {
            if let Some(j) = j {
                b.patch_jump(*j, body_starts[i]);
            }
        }
        match default_idx {
            Some(i) => b.patch_jump(no_match_jump, body_starts[i]),
            None => b.patch_jump(no_match_jump, end),
        }
        let ctx = self.loops.pop().unwrap();
        for br in ctx.breaks {
            b.patch_jump(br, end);
        }
        self.emit_pop_scope(b);
        self.redispatch_after_loop(b);
        Ok(())
    }

    fn compile_try(
        &mut self,
        b: &mut ChunkBuilder,
        block: &[Stmt],
        handler: &Option<(Option<Expr>, Vec<Stmt>)>,
        finalizer: &Option<Vec<Stmt>>,
    ) -> Result<(), String> {
        let block_chunk = self.compile_block_chunk(block)?;
        let handler_def = match handler {
            Some((param, body)) => {
                let param_name = match param {
                    Some(Expr::Ident(n)) => Some(n.clone()),
                    _ => None,
                };
                let hbody = self.compile_block_chunk(body)?;
                Some((param_name, hbody))
            }
            None => None,
        };
        let final_chunk = match finalizer {
            Some(f) => Some(self.compile_block_chunk(f)?),
            None => None,
        };
        let id = self.tries.len();
        self.tries.push(TryDef {
            block: block_chunk,
            handler: handler_def,
            finalizer: final_chunk,
        });
        b.emit(Op::LoadInt(id as i64), 0);
        b.emit(Op::CallBuiltin(ops::TRY, 1), 0);
        b.emit(Op::Pop, 0);
        // The try/catch/finally bodies ran as their own chunks, so a `return` or
        // a `break`/`continue` inside them left a signal instead of jumping.
        self.emit_signal_dispatch(b);
        Ok(())
    }

    /// Compile statements into a SEPARATE chunk (a try/catch/finally body). Loops
    /// opened outside it are unreachable by a plain jump, so `chunk_loop_base`
    /// moves up for the duration.
    fn compile_block_chunk(&mut self, stmts: &[Stmt]) -> Result<Chunk, String> {
        let mut cb = ChunkBuilder::new();
        // A nested chunk runs on its OWN VM frame, so the enclosing chunk's
        // slots are not reachable from it — everything here goes by name. (The
        // slot analysis already refuses any chunk containing a `try`, which is
        // what builds these; this keeps that true if another one appears.)
        let saved_slot_table = std::mem::take(&mut self.slots);
        let base = std::mem::replace(&mut self.chunk_loop_base, self.loops.len());
        let signals = std::mem::take(&mut self.chunk_signals);
        let depth = std::mem::take(&mut self.scope_depth);
        let iters = std::mem::take(&mut self.iter_depth);
        let sites = std::mem::take(&mut self.call_sites);
        let yields = std::mem::take(&mut self.yield_sites);
        let r = (|| {
            self.hoist_funcs(&mut cb, stmts)?;
            self.compile_stmts(&mut cb, stmts)
        })();
        self.chunk_loop_base = base;
        self.scope_depth = depth;
        self.iter_depth = iters;
        self.slots = saved_slot_table;
        // A signal raised inside the nested chunk still has to be dispatched by a
        // loop in THIS chunk, so the flag propagates outward.
        self.chunk_signals |= signals;
        r?;
        let chunk = self.finish_chunk(cb);
        self.call_sites = sites;
        self.yield_sites = yields;
        Ok(chunk)
    }

    // ── functions ────────────────────────────────────────────────────────
    fn build_function(
        &mut self,
        name: &str,
        params: &[Param],
        body: &[Stmt],
        is_generator: bool,
        is_async: bool,
    ) -> Result<usize, String> {
        let (param_slots, prologue) = self.lower_params(params)?;
        let mut fb = ChunkBuilder::new();
        // Each function body is its own frame, so it gets its own slot table.
        // The analysis sees the parameter prologue (defaults, destructuring)
        // ahead of the body, which is the order they are emitted in.
        let mut planned: Vec<Stmt> = prologue.clone();
        planned.extend_from_slice(body);
        let saved_slot_table = std::mem::replace(
            &mut self.slots,
            if self.debug || is_generator || is_async {
                Default::default()
            } else {
                crate::slots::plan(params, &planned, false)
            },
        );
        // Prologue: a parameter arrives in the call environment (`bind_params`
        // ran before this chunk), so copy each slotted one into its slot once,
        // and everything after it is a bare `GetSlot`.
        for name in crate::slots::param_names(params) {
            if let Some(slot) = self.slot_of(&name) {
                self.name_const(&mut fb, &name);
                fb.emit(Op::CallBuiltin(ops::GETLOCAL, 1), 0);
                fb.emit(Op::SetSlot(slot), 0);
            }
        }
        // A function body is its own control-flow universe: `break`/`continue` can
        // never target a loop in the enclosing function.
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_base = std::mem::replace(&mut self.chunk_loop_base, 0);
        let saved_signals = std::mem::take(&mut self.chunk_signals);
        let saved_depth = std::mem::take(&mut self.scope_depth);
        let saved_iters = std::mem::take(&mut self.iter_depth);
        let saved_agen = std::mem::replace(&mut self.in_async_generator, is_generator && is_async);
        // Strictness is inherited by every nested function and can only be
        // ADDED by a body's own directive prologue — never dropped.
        let saved_strict = self.strict;
        self.strict = self.strict || has_use_strict(body);
        // Captured before the restore below, since the FuncDef is built after
        // `self.strict` has been put back to the enclosing value.
        let body_strict = self.strict;
        // The body is a chunk of its own, so its call sites are keyed to ITS
        // `op_hash`; the enclosing chunk's pending ones must not be swept in.
        let saved_sites = std::mem::take(&mut self.call_sites);
        let saved_yields = std::mem::take(&mut self.yield_sites);
        let r = (|| {
            // Function-body hoisting: `var` bindings first, so a same-named
            // function declaration below overwrites the `undefined` rather than
            // being overwritten by it. Parameters are already bound, and
            // `hoist_var_name` leaves an existing binding alone.
            self.hoist_vars(&mut fb, body)?;
            self.hoist_funcs(&mut fb, &prologue)?;
            self.hoist_funcs(&mut fb, body)?;
            self.compile_stmts(&mut fb, &prologue)?;
            self.compile_stmts(&mut fb, body)
        })();
        self.loops = saved_loops;
        self.chunk_loop_base = saved_base;
        self.chunk_signals = saved_signals;
        self.scope_depth = saved_depth;
        self.iter_depth = saved_iters;
        self.in_async_generator = saved_agen;
        self.strict = saved_strict;
        self.slots = saved_slot_table;
        r?;
        let def = FuncDef {
            name: name.to_string(),
            params: param_slots,
            chunk: self.finish_chunk(fb),
            is_arrow: false,
            is_generator,
            is_async,
            is_method: false,
            self_name: false,
            strict: body_strict,
        };
        self.call_sites = saved_sites;
        self.yield_sites = saved_yields;
        self.functions.push((name.to_string(), def));
        Ok(self.functions.len() - 1)
    }

    fn build_arrow(
        &mut self,
        params: &[Param],
        body: &FnBody,
        is_async: bool,
    ) -> Result<usize, String> {
        let stmts = match body {
            FnBody::Block(b) => b.clone(),
            FnBody::Expr(e) => vec![Stmt::from(StmtKind::Return(Some((**e).clone())))],
        };
        let id = self.build_function("", params, &stmts, false, is_async)?;
        // Mark the template as an arrow so `this` is captured lexically.
        self.functions[id].1.is_arrow = true;
        Ok(id)
    }

    // ── classes ──────────────────────────────────────────────────────────
    /// Lower a `class` to runtime builder ops, leaving the class value on the
    /// stack: `MKCLASS` (name, parent, ctor) then `DEF_MEMBER`/`DEF_FIELD` for
    /// each member (each keeps the class on the stack).
    fn compile_class(&mut self, b: &mut ChunkBuilder, node: &ClassNode) -> Result<(), String> {
        // A class body is strict code unconditionally (10.2.4), directive or not.
        let saved_strict = std::mem::replace(&mut self.strict, true);
        let r = self.compile_class_body(b, node);
        self.strict = saved_strict;
        r
    }

    /// `#name` when this member's key is a literal private name, else `None`. A
    /// private name is never computed, so a computed key is never one.
    fn private_key(m: &ClassMember) -> Option<String> {
        match &m.key {
            Expr::Str(s) if !m.computed && s.starts_with('#') => Some(s.clone()),
            Expr::Ident(s) if !m.computed && s.starts_with('#') => Some(s.clone()),
            _ => None,
        }
    }

    fn compile_class_body(&mut self, b: &mut ChunkBuilder, node: &ClassNode) -> Result<(), String> {
        let cname = node.name.clone().unwrap_or_default();
        // Push name, parent (or undefined), constructor (or undefined).
        self.name_const(b, &cname);
        match &node.parent {
            Some(p) => self.compile_expr(b, p)?,
            None => {
                b.emit(Op::LoadUndef, 0);
            }
        }
        let ctor = node
            .members
            .iter()
            .find(|m| m.kind == MemberKind::Constructor);
        match ctor {
            Some(m) => {
                let def_id = self.build_function(&cname, &m.params, &m.body, false, false)?;
                self.emit_mkfunc(b, def_id);
            }
            None => {
                b.emit(Op::LoadUndef, 0);
            }
        }
        b.emit(Op::CallBuiltin(ops::MKCLASS, 3), 0); // -> [class]

        // 15.7.14 steps 8-17: the class body runs inside its OWN environment,
        // holding one immutable binding for the class name, initialized to the
        // class itself at step 17 — before the static-field initializers of step
        // 32. So `class C { static x = C.m(); static m(){return 5} }` is 5, and a
        // class EXPRESSION's name (`const K = class Inner { static s = Inner.name }`)
        // is reachable from inside the body even though it is never a binding
        // outside it. node-js had no such scope: both threw `ReferenceError: C is
        // not defined`, because the only binding was the outer one the class
        // DECLARATION installs afterwards. An instance method's body already
        // worked, but only by accident — it runs late enough for the outer
        // binding to exist, which a class expression never gets.
        let body_scope = node.name.is_some();
        if let Some(name) = &node.name {
            self.emit_push_scope(b);
            b.emit(Op::Dup, 0); // [class, class]
            self.declare_as(b, &Expr::Ident(name.clone()), BindMode::Lexical); // [class]
        }

        // `ClassDefinitionEvaluation` (15.7.14) installs every method and
        // accessor while evaluating the class body, and only then runs the
        // static-field initializers (step 32). So a static field may call a
        // static method declared after it, and `getOwnPropertyNames(C)` lists
        // the methods before the fields regardless of source order.
        // A `static { … }` block is a static ELEMENT, not a method: it belongs in
        // the deferred group with the field initializers and runs interleaved
        // with them in source order (both filters are stable over `members`).
        let deferred = |k: &MemberKind| matches!(k, MemberKind::Field | MemberKind::StaticBlock);
        let ordered = node
            .members
            .iter()
            .filter(|m| !deferred(&m.kind))
            .chain(node.members.iter().filter(|m| deferred(&m.kind)));
        let mut static_block_n = 0usize;
        for m in ordered {
            match m.kind {
                MemberKind::Constructor => {}
                // A PRIVATE static field declares a private element, so it
                // cannot be an ordinary write: `C.#s = 5` through `SETATTR`
                // trips the brand check that exists to reject exactly that write
                // on an object that has not declared `#s`. `DEF_MEMBER` installs
                // it directly, which is what a declaration is.
                MemberKind::Field if m.is_static && Self::private_key(m).is_some() => {
                    let key = Self::private_key(m).expect("guarded above");
                    self.name_const(b, &key); // [class, name]
                    b.emit(Op::LoadInt(member::STATIC_FIELD), 0);
                    b.emit(Op::LoadTrue, 0); // is_static
                    match &m.field_init {
                        Some(e) => self.emit_keyed_value(b, &m.key, e, false, member::METHOD)?,
                        None => {
                            b.emit(Op::LoadUndef, 0);
                        }
                    }
                    b.emit(Op::CallBuiltin(ops::DEF_MEMBER, 5), 0); // -> [class]
                }
                MemberKind::Field if m.is_static => {
                    // A static field is evaluated once at class-definition time and
                    // set as an own property of the constructor: `[class]` stays on
                    // the stack, `Dup` it as the SETATTR receiver.
                    b.emit(Op::Dup, 0); // [class, class]
                    self.emit_member_key(b, m)?; // [class, class, name]
                    match &m.field_init {
                        // 15.7.10: a static field's initializer is named after
                        // the field (`static s = function(){}` → `s`).
                        Some(e) => {
                            self.emit_keyed_value(b, &m.key, e, m.computed, member::METHOD)?
                        }
                        None => {
                            b.emit(Op::LoadUndef, 0);
                        }
                    }
                    // [class, class, name, val] -> SETATTR sets on the class -> [class, val]
                    b.emit(Op::CallBuiltin(ops::SETATTR, 3), 0);
                    b.emit(Op::Pop, 0); // drop the returned value -> [class]
                }
                MemberKind::Field => {
                    // [class] name thunk name_anon -> DEF_FIELD -> [class]
                    self.emit_member_key(b, m)?;
                    let init = m.field_init.clone().unwrap_or(Expr::Undefined);
                    // 15.7.10: `class C { f = function(){} }` names the function
                    // `f`. An instance field's initializer runs per-instance from
                    // a thunk, and under a computed key the key is only known at
                    // class-definition time, so the decision travels to the host
                    // as a flag rather than as an emitted rename.
                    let name_anon = Self::is_anon_fn_def(&init);
                    let stmts = vec![Stmt::from(StmtKind::Return(Some(init)))];
                    let def_id = self.build_function("", &[], &stmts, false, false)?;
                    self.emit_mkfunc(b, def_id);
                    b.emit(
                        if name_anon {
                            Op::LoadTrue
                        } else {
                            Op::LoadFalse
                        },
                        0,
                    );
                    b.emit(Op::CallBuiltin(ops::DEF_FIELD, 4), 0);
                }
                MemberKind::StaticBlock => {
                    // `static { … }` runs ONCE at class-definition time with
                    // `this` bound to the constructor — exactly what a static
                    // method called as `C.m()` gets. So it is compiled as a
                    // static method under a HIDDEN key, invoked, and removed
                    // again; the `@@` prefix keeps it out of every enumeration
                    // (`Object.getOwnPropertyNames(C)` and friends filter
                    // internal slots) for the window in which it exists, and the
                    // counter keeps sibling blocks from colliding.
                    static_block_n += 1;
                    let slot = format!("@@staticBlock:{static_block_n}");
                    // [class] name kind static fn -> DEF_MEMBER -> [class]
                    self.name_const(b, &slot);
                    b.emit(Op::LoadInt(member::METHOD), 0);
                    b.emit(Op::LoadTrue, 0);
                    let def_id = self.build_function("", &[], &m.body, false, false)?;
                    self.functions[def_id].1.is_method = true;
                    self.emit_mkfunc(b, def_id);
                    b.emit(Op::CallBuiltin(ops::DEF_MEMBER, 5), 0);
                    // [class] -> C[slot]() -> discard the result
                    b.emit(Op::Dup, 0);
                    self.name_const(b, &slot);
                    b.emit(Op::CallBuiltin(ops::CALL_METHOD, 2), 0);
                    b.emit(Op::Pop, 0);
                    // [class] -> delete C[slot] -> discard the Bool
                    b.emit(Op::Dup, 0);
                    self.name_const(b, &slot);
                    b.emit(Op::CallBuiltin(ops::DELPROP_NAME, 2), 0);
                    b.emit(Op::Pop, 0);
                }
                MemberKind::Method | MemberKind::Get | MemberKind::Set => {
                    // [class] name kind static fn -> DEF_MEMBER -> [class]
                    self.emit_member_key(b, m)?;
                    let kind = match m.kind {
                        MemberKind::Get => member::GET,
                        MemberKind::Set => member::SET,
                        _ => member::METHOD,
                    };
                    b.emit(Op::LoadInt(kind), 0);
                    b.emit(
                        if m.is_static {
                            Op::LoadTrue
                        } else {
                            Op::LoadFalse
                        },
                        0,
                    );
                    // 10.2.9 step 4: an accessor's function name carries the
                    // `get `/`set ` prefix — `class C { get gg(){} }` gives
                    // `get gg`, not `gg`.
                    let mname = match &m.key {
                        Expr::Str(s) if !m.computed => match m.kind {
                            MemberKind::Get => format!("get {s}"),
                            MemberKind::Set => format!("set {s}"),
                            _ => s.clone(),
                        },
                        _ => String::new(),
                    };
                    let def_id = self.build_function(
                        &mname,
                        &m.params,
                        &m.body,
                        m.is_generator,
                        m.is_async,
                    )?;
                    // A class method/accessor is a MethodDefinition: not a
                    // constructor, so it owns no `prototype` property.
                    self.functions[def_id].1.is_method = true;
                    self.emit_mkfunc(b, def_id);
                    b.emit(Op::CallBuiltin(ops::DEF_MEMBER, 5), 0);
                }
            }
        }
        if body_scope {
            self.emit_pop_scope(b);
        }
        Ok(())
    }

    /// `IsAnonymousFunctionDefinition(expr)` — the SYNTACTIC predicate that
    /// decides whether NamedEvaluation applies. It is deliberately not a runtime
    /// "does this function have an empty name" test: measured against node
    /// v26.7.0, `const anon = (0, function(){}); ({ m: anon }).m.name` is `""`,
    /// because the property definition's right-hand side is an
    /// IdentifierReference, not a function definition. Renaming by value would
    /// also mutate a function the program still holds under another binding.
    fn is_anon_fn_def(init: &Expr) -> bool {
        match init {
            Expr::Function { name: None, .. } => true,
            Expr::Class(node) => node.name.is_none(),
            _ => false,
        }
    }

    /// If `init` is an anonymous function/arrow/class (value already on TOS), set
    /// its `.name` to `name` (JS binding name-inference). No-op otherwise.
    fn infer_name(&mut self, b: &mut ChunkBuilder, init: &Expr, name: &str) {
        if !Self::is_anon_fn_def(init) {
            return;
        }
        // [fn] Dup; .name = name; drop the SETATTR result.
        b.emit(Op::Dup, 0);
        self.name_const(b, "name");
        self.strlit(b, name);
        b.emit(Op::CallBuiltin(ops::SETATTR, 3), 0);
        b.emit(Op::Pop, 0);
    }

    /// Compile a member's VALUE with the key already on the stack, applying
    /// NamedEvaluation (10.2.9 SetFunctionName) when the value is an anonymous
    /// function definition — `{ m: function(){} }`, `{ m(){} }`, `{ [k]: () => {} }`,
    /// `class C { static [k] = function(){} }`.
    ///
    /// A literal key resolves at compile time; a computed one is only known at
    /// run time, so the key already on the stack is duplicated and handed to
    /// `NAMED_EVAL` along with `kind` (which supplies the `get `/`set ` prefix).
    /// Leaves exactly one value on the stack either way, so every caller's
    /// arity is unchanged.
    fn emit_keyed_value(
        &mut self,
        b: &mut ChunkBuilder,
        key: &Expr,
        value: &Expr,
        computed: bool,
        kind: i64,
    ) -> Result<(), String> {
        match (Self::is_anon_fn_def(value), computed, key) {
            (true, false, Expr::Str(s)) => {
                self.compile_expr(b, value)?;
                let name = match kind {
                    member::GET => format!("get {s}"),
                    member::SET => format!("set {s}"),
                    _ => s.clone(),
                };
                self.infer_name(b, value, &name);
            }
            // [.., key] -> [.., key, key, kind, fn] -> NAMED_EVAL -> [.., key, fn]
            (true, true, _) => {
                b.emit(Op::Dup, 0);
                b.emit(Op::LoadInt(kind), 0);
                self.compile_expr(b, value)?;
                b.emit(Op::CallBuiltin(ops::NAMED_EVAL, 3), 0);
            }
            _ => self.compile_expr(b, value)?,
        }
        Ok(())
    }

    /// Push a class/object member's property key: a computed expression coerced
    /// via `PROPKEY` (Symbol-aware), or a static name constant.
    fn emit_member_key(&mut self, b: &mut ChunkBuilder, m: &ClassMember) -> Result<(), String> {
        if m.computed {
            self.compile_expr(b, &m.key)?;
            b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
        } else if let Expr::Str(s) = &m.key {
            self.name_const(b, s);
        } else {
            self.compile_expr(b, &m.key)?;
            b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
        }
        Ok(())
    }

    // ── generators / yield ───────────────────────────────────────────────
    fn compile_yield(
        &mut self,
        b: &mut ChunkBuilder,
        arg: &Option<Box<Expr>>,
        delegate: bool,
    ) -> Result<(), String> {
        if delegate && self.in_async_generator {
            // `yield* x` inside an `async function*` delegates over the ASYNC
            // iterator: await each step, re-yield its value, and evaluate to the
            // delegate's return value.
            match arg {
                Some(e) => self.compile_expr(b, e)?,
                None => {
                    b.emit(Op::LoadUndef, 0);
                }
            }
            b.emit(Op::CallBuiltin(ops::GET_ASYNC_ITER, 1), 0); // [aiter]
            let start = b.current_pos();
            b.emit(Op::Dup, 0); // [aiter, aiter]
            b.emit(Op::CallBuiltin(ops::ASYNC_STEP, 1), 0); // [aiter, stepPromise]
            b.emit(Op::CallBuiltin(ops::AWAIT, 1), 0); // [aiter, step]
            b.emit(Op::Dup, 0); // [aiter, step, step]
            self.name_const(b, "done");
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
            b.emit(Op::CallBuiltin(ops::TRUTHY, 1), 0);
            let jdone = b.emit(Op::JumpIfTrue(0), 0); // [aiter, step]
            self.name_const(b, "value");
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [aiter, value]
            let at = b.emit(Op::CallBuiltin(ops::YIELD, 1), 0); // [aiter, sent]
            self.yield_sites.push((at, self.iter_depth));
            b.emit(Op::Pop, 0); // [aiter]
            b.emit(Op::Jump(start), 0);
            let done = b.current_pos();
            b.patch_jump(jdone, done);
            self.name_const(b, "value"); // [aiter, step, "value"]
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [aiter, returnValue]
            b.emit(Op::Swap, 0); // [returnValue, aiter]
            b.emit(Op::Pop, 0); // [returnValue]
        } else if delegate {
            // `yield* iterable`: step the delegate through the iterator protocol,
            // re-yielding each value and FORWARDING whatever `.next(x)` sent in.
            // The expression's value is the delegate's RETURN value, which
            // `FORITER` discards — hence the explicit `.next()` calls.
            let sent_tmp = self.tmp_name("delegated");
            match arg {
                Some(e) => self.compile_expr(b, e)?,
                None => {
                    b.emit(Op::LoadUndef, 0);
                }
            }
            b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
                                                         // The delegate is parked on the stack for the whole delegation, so
                                                         // it counts as a live iterator: a `.return()`/`.throw()` injected
                                                         // into the OUTER generator has to close it (7.4.9 IteratorClose),
                                                         // which is what runs the delegate's pending `finally`.
            self.iter_depth += 1;
            self.name_const(b, &sent_tmp);
            b.emit(Op::LoadUndef, 0);
            b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
            b.emit(Op::Pop, 0);
            let start = b.current_pos();
            b.emit(Op::Dup, 0); // [iterator, iterator]
            self.name_const(b, "next");
            self.load_local(b, &sent_tmp);
            b.emit(Op::CallBuiltin(ops::CALL_METHOD, 3), 0); // [iterator, step]
            b.emit(Op::Dup, 0);
            self.name_const(b, "done");
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
            b.emit(Op::CallBuiltin(ops::TRUTHY, 1), 0);
            let jdone = b.emit(Op::JumpIfTrue(0), 0); // [iterator, step]
            self.name_const(b, "value");
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [iterator, value]
            let at = b.emit(Op::CallBuiltin(ops::YIELD, 1), 0); // [iterator, sent]
            self.yield_sites.push((at, self.iter_depth));
            self.name_const(b, &sent_tmp);
            b.emit(Op::Swap, 0);
            b.emit(Op::CallBuiltin(ops::SETLOCAL, 2), 0);
            b.emit(Op::Pop, 0);
            b.emit(Op::Jump(start), 0);
            let done = b.current_pos();
            b.patch_jump(jdone, done);
            self.name_const(b, "value"); // [iterator, step, "value"]
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [iterator, returnValue]
            b.emit(Op::Swap, 0);
            b.emit(Op::Pop, 0); // [returnValue]
            self.iter_depth -= 1;
        } else {
            match arg {
                Some(e) => self.compile_expr(b, e)?,
                None => {
                    b.emit(Op::LoadUndef, 0);
                }
            }
            // YIELD suspends and leaves the value sent by `.next(x)` on the stack.
            let at = b.emit(Op::CallBuiltin(ops::YIELD, 1), 0);
            self.yield_sites.push((at, self.iter_depth));
        }
        Ok(())
    }

    /// Lower a formal-parameter list into simple slots plus prologue statements
    /// (defaults + destructuring), executed at the top of the body.
    fn lower_params(&mut self, params: &[Param]) -> Result<(Vec<ParamSlot>, Vec<Stmt>), String> {
        let mut slots = Vec::new();
        let mut prologue: Vec<Stmt> = Vec::new();
        for (i, p) in params.iter().enumerate() {
            if p.rest {
                let name = match &p.pattern {
                    Expr::Ident(n) => n.clone(),
                    _ => return Err("SyntaxError: rest parameter must be an identifier".into()),
                };
                slots.push(ParamSlot {
                    name,
                    rest: true,
                    has_default: false,
                });
                continue;
            }
            match &p.pattern {
                Expr::Ident(name) => {
                    slots.push(ParamSlot {
                        name: name.clone(),
                        rest: false,
                        has_default: p.default.is_some(),
                    });
                    if let Some(d) = &p.default {
                        prologue.push(default_stmt(name, d));
                    }
                }
                pattern => {
                    let synth = format!(".param{i}");
                    slots.push(ParamSlot {
                        name: synth.clone(),
                        rest: false,
                        has_default: p.default.is_some(),
                    });
                    if let Some(d) = &p.default {
                        prologue.push(default_stmt(&synth, d));
                    }
                    prologue.push(Stmt::from(StmtKind::Decl {
                        kind: DeclKind::Let,
                        decls: vec![Declarator {
                            target: pattern.clone(),
                            init: Some(Expr::Ident(synth)),
                        }],
                    }));
                }
            }
        }
        Ok((slots, prologue))
    }

    // ── expressions ──────────────────────────────────────────────────────
    fn compile_expr(&mut self, b: &mut ChunkBuilder, e: &Expr) -> Result<(), String> {
        match e {
            Expr::Undefined => {
                b.emit(Op::LoadUndef, 0);
            }
            // A hole only carries its extra meaning INSIDE an array literal
            // (`compile_array` records it); evaluated anywhere else it is just
            // the `undefined` an elided read produces.
            Expr::Hole => {
                b.emit(Op::LoadUndef, 0);
            }
            Expr::Null => {
                b.emit(Op::CallBuiltin(ops::LOAD_NULL, 0), 0);
            }
            Expr::True => {
                b.emit(Op::LoadTrue, 0);
            }
            Expr::False => {
                b.emit(Op::LoadFalse, 0);
            }
            Expr::Number(n) => {
                b.emit(Op::LoadFloat(*n), 0);
            }
            Expr::BigInt(digits) => {
                // The canonical decimal digit string travels as a native constant;
                // MKBIGINT parses it into a heap BigInt at runtime.
                let k = b.add_constant(Value::str(digits));
                b.emit(Op::LoadConst(k), 0);
                b.emit(Op::CallBuiltin(ops::MKBIGINT, 1), 0);
            }
            Expr::Regex(pat, flags) => {
                let kp = b.add_constant(Value::str(pat));
                b.emit(Op::LoadConst(kp), 0);
                let kf = b.add_constant(Value::str(flags));
                b.emit(Op::LoadConst(kf), 0);
                b.emit(Op::CallBuiltin(ops::MKREGEX, 2), 0);
            }
            Expr::Str(s) => self.strlit(b, s),
            Expr::Template { quasis, exprs } => self.compile_template(b, quasis, exprs)?,
            Expr::TaggedTemplate {
                tag,
                quasis,
                raws,
                exprs,
            } => self.compile_tagged_template(b, tag, quasis, raws, exprs)?,
            Expr::Ident(n) => self.load_local(b, n),
            Expr::This => {
                b.emit(Op::CallBuiltin(ops::THIS, 0), 0);
            }
            Expr::Array(items) => self.compile_array(b, items)?,
            Expr::Object(props) => self.compile_object(b, props)?,
            Expr::Spread(inner) => self.compile_expr(b, inner)?,
            Expr::Logical(op, l, r) => self.compile_logical(b, *op, l, r)?,
            Expr::Unary(op, e) => self.compile_unary(b, *op, e)?,
            Expr::Binary(op, l, r) => self.compile_binary(b, *op, l, r)?,
            Expr::Conditional { test, cons, alt } => {
                self.compile_condition(b, test)?;
                let jf = b.emit(Op::JumpIfFalse(0), 0);
                self.compile_expr(b, cons)?;
                let je = b.emit(Op::Jump(0), 0);
                let els = b.current_pos();
                b.patch_jump(jf, els);
                self.compile_expr(b, alt)?;
                let end = b.current_pos();
                b.patch_jump(je, end);
            }
            Expr::Assign { target, value } => {
                self.compile_expr(b, value)?;
                // 13.15.2 step 1.e: `h = function(){}` names the function `h`.
                // Only an IdentifierReference target counts — `o.p = function(){}`
                // leaves the name empty in node too.
                if let Expr::Ident(n) = &**target {
                    self.infer_name(b, value, n);
                }
                b.emit(Op::Dup, 0); // assignment yields the value
                self.compile_bind(b, target, BindMode::Assign)?;
            }
            Expr::Update { op, prefix, target } => self.compile_update(b, *op, *prefix, target)?,
            // A chain's ROOT opens the frame its `?.` links park their jumps
            // in; nested links see it already open and add to it.
            Expr::Call { .. } | Expr::Member { .. } | Expr::Index { .. }
                if self.opt_chain.is_empty() && Self::spine_has_optional(e) =>
            {
                self.compile_chain_root(b, e)?
            }
            Expr::Call {
                func,
                args,
                optional,
            } => self.compile_call(b, func, args, *optional)?,
            Expr::New { callee, args } => self.compile_new(b, callee, args)?,
            Expr::Member {
                object,
                property,
                optional,
            } => self.compile_member(b, object, property, *optional)?,
            Expr::Index {
                object,
                index,
                optional,
            } => self.compile_index(b, object, index, *optional)?,
            Expr::Function {
                params,
                body,
                is_arrow,
                name,
                is_generator,
                is_async,
                is_method,
            } => {
                let def_id = if *is_arrow {
                    self.build_arrow(params, body, *is_async)?
                } else {
                    let n = name.clone().unwrap_or_default();
                    let stmts = match body {
                        FnBody::Block(b) => b.clone(),
                        FnBody::Expr(e) => vec![Stmt::from(StmtKind::Return(Some((**e).clone())))],
                    };
                    let id = self.build_function(&n, params, &stmts, *is_generator, *is_async)?;
                    // A NAMED function expression binds its own name inside the body
                    // (object/class methods parse with `name: None`, so this only
                    // fires for `function name(…) {…}` in expression position).
                    if name.is_some() {
                        self.functions[id].1.self_name = true;
                    }
                    self.functions[id].1.is_method = *is_method;
                    id
                };
                self.emit_mkfunc(b, def_id);
            }
            Expr::Class(node) => self.compile_class(b, node)?,
            Expr::Super => {
                // Bare `super` only appears as a call/member callee, handled by
                // compile_call / compile_member; a stray `super` yields undefined.
                b.emit(Op::LoadUndef, 0);
            }
            Expr::NewTarget => {
                b.emit(Op::CallBuiltin(ops::NEW_TARGET, 0), 0);
            }
            Expr::Yield { arg, delegate } => self.compile_yield(b, arg, *delegate)?,
            Expr::Await(inner) => {
                self.compile_expr(b, inner)?;
                b.emit(Op::CallBuiltin(ops::AWAIT, 1), 0);
            }
            Expr::Sequence(items) => {
                for (i, it) in items.iter().enumerate() {
                    self.compile_expr(b, it)?;
                    if i + 1 < items.len() {
                        b.emit(Op::Pop, 0);
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_template(
        &mut self,
        b: &mut ChunkBuilder,
        quasis: &[String],
        exprs: &[Expr],
    ) -> Result<(), String> {
        let mut n = 0;
        for (i, q) in quasis.iter().enumerate() {
            let k = b.add_constant(Value::str(q));
            b.emit(Op::LoadConst(k), 0);
            n += 1;
            if i < exprs.len() {
                self.compile_expr(b, &exprs[i])?;
                b.emit(Op::CallBuiltin(ops::TOSTR, 1), 0);
                n += 1;
            }
        }
        b.emit(Op::CallBuiltin(ops::MKSTR, argc(n)?), 0);
        Ok(())
    }

    /// Lower a tagged template to `TAG_TMPL`. Operand layout (matching
    /// `builtins::b_tag_tmpl`): `[tag, n, m, cooked×n, raw×n, values×m]`, where
    /// `n = quasis.len()` and `m = exprs.len()` (`n == m + 1`).
    fn compile_tagged_template(
        &mut self,
        b: &mut ChunkBuilder,
        tag: &Expr,
        quasis: &[String],
        raws: &[String],
        exprs: &[Expr],
    ) -> Result<(), String> {
        self.compile_expr(b, tag)?;
        let n = quasis.len();
        let m = exprs.len();
        b.emit(Op::LoadInt(n as i64), 0);
        b.emit(Op::LoadInt(m as i64), 0);
        for q in quasis {
            self.strlit(b, q); // cooked strings (heap)
        }
        for r in raws {
            self.strlit(b, r); // raw strings (heap)
        }
        for e in exprs {
            self.compile_expr(b, e)?; // substitution values
        }
        b.emit(Op::CallBuiltin(ops::TAG_TMPL, argc(3 + 2 * n + m)?), 0);
        Ok(())
    }

    fn compile_array(&mut self, b: &mut ChunkBuilder, items: &[Expr]) -> Result<(), String> {
        if items.iter().any(|e| matches!(e, Expr::Spread(_))) {
            // (tag, value) pairs; tag 1 = spread, tag 2 = elision. A spread
            // makes every later element's index a RUN-TIME quantity, so the
            // holes cannot be recorded from here — the tag carries the fact and
            // `BUILD_ARGS` marks them as it walks.
            for it in items {
                match it {
                    Expr::Spread(inner) => {
                        b.emit(Op::LoadInt(1), 0);
                        self.compile_expr(b, inner)?;
                    }
                    Expr::Hole => {
                        b.emit(Op::LoadInt(2), 0);
                        b.emit(Op::LoadUndef, 0);
                    }
                    _ => {
                        b.emit(Op::LoadInt(0), 0);
                        self.compile_expr(b, it)?;
                    }
                }
            }
            b.emit(Op::CallBuiltin(ops::BUILD_ARGS, argc(items.len() * 2)?), 0);
        } else if items.len() <= u8::MAX as usize {
            for it in items {
                self.compile_expr(b, it)?;
            }
            b.emit(Op::CallBuiltin(ops::MKARR, argc(items.len())?), 0);
            self.mark_literal_holes(b, items);
        } else {
            // A literal larger than one CallBuiltin's u8 arg count can hold (the
            // generated data tables in iconv-lite hit this): start from an empty
            // array and append each element with an indexed store, keeping the
            // array on the stack across iterations.
            b.emit(Op::CallBuiltin(ops::MKARR, 0), 0); // [arr]
            for (i, it) in items.iter().enumerate() {
                b.emit(Op::Dup, 0); // [arr, arr]
                b.emit(Op::LoadInt(i as i64), 0); // [arr, arr, i]
                self.compile_expr(b, it)?; // [arr, arr, i, val]
                b.emit(Op::CallBuiltin(ops::SETITEM, 3), 0); // -> [arr, val]
                b.emit(Op::Pop, 0); // [arr]
            }
            // After the writes: a `SETITEM` CLEARS the hole at the index it
            // writes, so marking has to come last.
            self.mark_literal_holes(b, items);
        }
        Ok(())
    }

    /// Emit a `MARK_HOLE` per elided position of a spread-free array literal,
    /// with the finished array on top of the stack. Emits nothing at all for the
    /// dense literals that are essentially every literal in real code.
    fn mark_literal_holes(&mut self, b: &mut ChunkBuilder, items: &[Expr]) {
        for (i, it) in items.iter().enumerate() {
            if !matches!(it, Expr::Hole) {
                continue;
            }
            b.emit(Op::Dup, 0); // [arr, arr]
            b.emit(Op::LoadInt(i as i64), 0); // [arr, arr, i]
            b.emit(Op::CallBuiltin(ops::MARK_HOLE, 2), 0); // [arr, undefined]
            b.emit(Op::Pop, 0); // [arr]
        }
    }

    fn compile_object(&mut self, b: &mut ChunkBuilder, props: &[Prop]) -> Result<(), String> {
        // (tag, key, val) triples for the data/spread props; tag 1 = ...spread.
        // Accessors are installed afterward via DEF_ACCESSOR.
        // An ACCESSOR keeps its slot in this list — with a tag of its own — so
        // the object enumerates it where the source declared it. The pair
        // `get`/`set` for one key contributes ONE slot.
        let mut seen_accessor: Vec<String> = Vec::new();
        let data: Vec<&Prop> = props
            .iter()
            .filter(|p| match p {
                Prop::Accessor { key, computed, .. } => {
                    // Only a literal key can be de-duplicated at compile time; a
                    // computed one is settled by `b_mkobj`'s `or_insert`.
                    let literal = match (key, computed) {
                        (Expr::Str(s), false) => Some(s.clone()),
                        _ => None,
                    };
                    match literal {
                        Some(k) if seen_accessor.contains(&k) => false,
                        Some(k) => {
                            seen_accessor.push(k);
                            true
                        }
                        None => true,
                    }
                }
                _ => true,
            })
            .collect();
        let has_spread = data.iter().any(|p| matches!(p, Prop::Spread(_)));
        // A spread-free literal with more triples than one CallBuiltin's u8 arg
        // count can hold (iconv-lite's generated codepage tables are 150+ keys)
        // is built incrementally: start empty, store each key, keeping the object
        // on the stack. Spread merges need the single-shot MKOBJ tag path, so
        // large-with-spread stays on it (a rare, genuine limitation).
        if data.len() * 3 > u8::MAX as usize && !has_spread {
            b.emit(Op::CallBuiltin(ops::MKOBJ, 0), 0); // [obj]
            for p in &data {
                if let Prop::KeyValue {
                    key,
                    value,
                    computed,
                } = p
                {
                    b.emit(Op::Dup, 0); // [obj, obj]
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0); // [obj, obj, key]
                    self.emit_keyed_value(b, key, value, *computed, member::METHOD)?;
                    b.emit(Op::CallBuiltin(ops::SETITEM, 3), 0); // -> [obj, val]
                    b.emit(Op::Pop, 0); // [obj]
                }
            }
            // The incremental path's accessors keep their trailing order: a
            // literal that large is a generated data table, and none carry one.
            return self.compile_object_accessors(b, props);
        }
        for p in &data {
            match p {
                Prop::KeyValue {
                    key,
                    value,
                    computed,
                } => {
                    // Tag 3 marks a METHOD DEFINITION, so `MKOBJ` can give it
                    // the literal as its `[[HomeObject]]`. It has to be decided
                    // HERE: a method assigned from elsewhere (`{ m: other.m }`)
                    // is an ordinary value whose home object was fixed where it
                    // was defined, and the runtime cannot tell the two apart
                    // from the value alone.
                    let defines_method = matches!(
                        value,
                        Expr::Function {
                            is_method: true,
                            ..
                        }
                    );
                    b.emit(Op::LoadInt(if defines_method { 3 } else { 0 }), 0);
                    // Key coerces to a property key (Symbol-aware: a Symbol maps to
                    // its internal `@@…` key rather than a `String()` coercion).
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                    self.emit_keyed_value(b, key, value, *computed, member::METHOD)?;
                }
                Prop::Spread(src) => {
                    b.emit(Op::LoadInt(1), 0);
                    self.compile_expr(b, src)?;
                    b.emit(Op::LoadUndef, 0);
                }
                // Reserve the accessor's enumeration slot; `DEF_ACCESSOR` below
                // installs the functions themselves.
                Prop::Accessor { key, computed, .. } => {
                    let _ = computed; // the key expression covers both forms
                    b.emit(Op::LoadInt(2), 0);
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                    b.emit(Op::LoadUndef, 0);
                }
            }
        }
        b.emit(Op::CallBuiltin(ops::MKOBJ, argc(data.len() * 3)?), 0); // [obj]
        self.compile_object_accessors(b, props)
    }

    /// Install any getter/setter accessors of an object literal onto the object
    /// left on the stack (shared by the single-shot and incremental build paths).
    fn compile_object_accessors(
        &mut self,
        b: &mut ChunkBuilder,
        props: &[Prop],
    ) -> Result<(), String> {
        for p in props {
            if let Prop::Accessor {
                key,
                computed,
                is_getter,
                func,
            } = p
            {
                if *computed {
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                } else if let Expr::Str(s) = key {
                    self.name_const(b, s);
                } else {
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                }
                let kind = if *is_getter { member::GET } else { member::SET };
                b.emit(Op::LoadInt(kind), 0);
                // `{ get g(){} }` names the getter `get g` (10.2.9 step 4 via
                // 13.2.5.5). A COMPUTED accessor key is the one member position
                // whose key is not still reachable on the stack here — `kind`
                // sits between it and the function — so it keeps the empty name.
                if *computed {
                    self.compile_expr(b, func)?;
                } else if let Expr::Str(s) = key {
                    self.compile_expr(b, func)?;
                    let prefix = if *is_getter { "get" } else { "set" };
                    self.infer_name(b, func, &format!("{prefix} {s}"));
                } else {
                    self.compile_expr(b, func)?;
                }
                b.emit(Op::CallBuiltin(ops::DEF_ACCESSOR, 4), 0);
            }
        }
        Ok(())
    }

    fn compile_logical(
        &mut self,
        b: &mut ChunkBuilder,
        op: LogicalOp,
        l: &Expr,
        r: &Expr,
    ) -> Result<(), String> {
        self.compile_expr(b, l)?;
        b.emit(Op::Dup, 0);
        let test_op = match op {
            LogicalOp::And | LogicalOp::Or => ops::TRUTHY,
            LogicalOp::Nullish => ops::NULLISH,
        };
        b.emit(Op::CallBuiltin(test_op, 1), 0);
        let jump = match op {
            LogicalOp::And => b.emit(Op::JumpIfFalse(0), 0), // false -> keep left
            LogicalOp::Or => b.emit(Op::JumpIfTrue(0), 0),   // true -> keep left
            LogicalOp::Nullish => b.emit(Op::JumpIfFalse(0), 0), // not-nullish -> keep left
        };
        b.emit(Op::Pop, 0); // drop left, evaluate right
        self.compile_expr(b, r)?;
        let end = b.current_pos();
        b.patch_jump(jump, end);
        Ok(())
    }

    fn compile_unary(&mut self, b: &mut ChunkBuilder, op: UnOp, e: &Expr) -> Result<(), String> {
        match op {
            UnOp::Neg => {
                self.compile_expr(b, e)?;
                b.emit(Op::Negate, 0);
            }
            UnOp::Not => {
                self.compile_condition(b, e)?;
                b.emit(Op::LogNot, 0);
            }
            UnOp::Pos => {
                b.emit(Op::LoadInt(unop::POS), 0);
                self.compile_expr(b, e)?;
                b.emit(Op::CallBuiltin(ops::UNARY, 2), 0);
            }
            UnOp::BitNot => {
                b.emit(Op::LoadInt(unop::BITNOT), 0);
                self.compile_expr(b, e)?;
                b.emit(Op::CallBuiltin(ops::UNARY, 2), 0);
            }
            UnOp::TypeOf => {
                // `typeof <bare ident>` must NOT throw when the name is unbound —
                // JS returns "undefined". Route a plain identifier through a
                // non-throwing name read; any other operand evaluates normally.
                if let Expr::Ident(n) = e {
                    // A slotted local is always bound by the time it is read
                    // (that is rule 3 of the slot analysis), so there is no
                    // unbound case for `TYPEOF_NAME` to absorb.
                    if let Some(slot) = self.slot_of(n) {
                        b.emit(Op::GetSlot(slot), 0);
                        b.emit(Op::CallBuiltin(ops::TYPEOF, 1), 0);
                        return Ok(());
                    }
                    self.name_const(b, n);
                    b.emit(Op::CallBuiltin(ops::TYPEOF_NAME, 1), 0);
                } else {
                    self.compile_expr(b, e)?;
                    b.emit(Op::CallBuiltin(ops::TYPEOF, 1), 0);
                }
            }
            UnOp::Void => {
                self.compile_expr(b, e)?;
                b.emit(Op::Pop, 0);
                b.emit(Op::LoadUndef, 0);
            }
            UnOp::Delete => match e {
                Expr::Member {
                    object, property, ..
                } => {
                    self.compile_expr(b, object)?;
                    self.name_const(b, property);
                    b.emit(Op::CallBuiltin(ops::DELPROP_NAME, 2), 0);
                }
                Expr::Index { object, index, .. } => {
                    self.compile_expr(b, object)?;
                    self.compile_expr(b, index)?;
                    b.emit(Op::CallBuiltin(ops::DELITEM, 2), 0);
                }
                _ => {
                    b.emit(Op::LoadTrue, 0);
                }
            },
        }
        Ok(())
    }

    fn compile_binary(
        &mut self,
        b: &mut ChunkBuilder,
        op: BinOp,
        l: &Expr,
        r: &Expr,
    ) -> Result<(), String> {
        // Native fast path (JIT-traceable); the numeric hook supplies JS
        // semantics for non-number operands.
        macro_rules! native {
            ($opc:expr) => {{
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit($opc, 0);
                return Ok(());
            }};
        }
        match op {
            BinOp::Add => native!(Op::Add),
            BinOp::Sub => native!(Op::Sub),
            BinOp::Mul => native!(Op::Mul),
            BinOp::Div => {
                // NOT native `Op::Div`: fusevm returns `Undef` for a zero divisor,
                // but JS needs `x/0 === ±Infinity` / `0/0 === NaN`, so `/` is a
                // builtin (fusevm's own documented pattern for non-default `/`).
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::DIV, 2), 0);
                return Ok(());
            }
            BinOp::Mod => native!(Op::Mod),
            // NOT native `Op::Pow`, for the same reason `/` is a builtin above:
            // fusevm's is IEEE-754 `pow`, where `(-1) ** Infinity` and `1 ** NaN`
            // come back 1 rather than the spec's NaN.
            BinOp::Pow => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::POW, 2), 0);
                return Ok(());
            }
            BinOp::Lt => native!(Op::NumLt),
            BinOp::Le => native!(Op::NumLe),
            BinOp::Gt => native!(Op::NumGt),
            BinOp::Ge => native!(Op::NumGe),
            BinOp::EqEqEq => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
            }
            BinOp::NeEqEq => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
                b.emit(Op::LogNot, 0);
            }
            BinOp::EqEq => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::LOOSE_EQ, 2), 0);
            }
            BinOp::NeEq => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::LOOSE_EQ, 2), 0);
                b.emit(Op::LogNot, 0);
            }
            BinOp::In => {
                // `#field in obj` is the private-brand check: the left operand is a
                // private NAME, not a variable read, so it lowers to the key string
                // (private fields live as `#`-prefixed properties on the instance).
                match l {
                    Expr::Ident(n) if n.starts_with('#') => self.name_const(b, n),
                    _ => self.compile_expr(b, l)?,
                }
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::CONTAINS, 2), 0);
            }
            BinOp::InstanceOf => {
                self.compile_expr(b, l)?;
                self.compile_expr(b, r)?;
                b.emit(Op::CallBuiltin(ops::INSTANCEOF, 2), 0);
            }
            BinOp::BitAnd => self.emit_bitwise(b, bop::BITAND, l, r)?,
            BinOp::BitOr => self.emit_bitwise(b, bop::BITOR, l, r)?,
            BinOp::BitXor => self.emit_bitwise(b, bop::BITXOR, l, r)?,
            BinOp::Shl => self.emit_bitwise(b, bop::SHL, l, r)?,
            BinOp::Shr => self.emit_bitwise(b, bop::SHR, l, r)?,
            BinOp::UShr => self.emit_bitwise(b, bop::USHR, l, r)?,
        }
        Ok(())
    }

    fn emit_bitwise(
        &mut self,
        b: &mut ChunkBuilder,
        tag: i64,
        l: &Expr,
        r: &Expr,
    ) -> Result<(), String> {
        b.emit(Op::LoadInt(tag), 0);
        self.compile_expr(b, l)?;
        self.compile_expr(b, r)?;
        b.emit(Op::CallBuiltin(ops::BINOP, 3), 0);
        Ok(())
    }

    fn compile_update(
        &mut self,
        b: &mut ChunkBuilder,
        op: UpdateOp,
        prefix: bool,
        target: &Expr,
    ) -> Result<(), String> {
        // `NUM_STEP(tag, old)` computes `ToNumeric(old)` and `old ± 1` preserving
        // the operand's numeric type — so `x++` on a BigInt stays a BigInt
        // (`+old`/`old + 1` would throw the mix error). It pushes the coerced old
        // value and returns the new value: stack `[tag, old]` → `[oldN, new]`.
        let tag = if matches!(op, UpdateOp::Inc) { 1 } else { -1 };
        // A slot that provably holds a Number needs none of that: `ToNumeric`
        // is the identity on it and `Number ± 1` is a Number, so the whole
        // update is `GetSlot`, a native `Add`, and `SetSlot`. This is what takes
        // the last `CallBuiltin` out of a counting loop's body — and with it the
        // reason fusevm's tiers decline the loop.
        if let Expr::Ident(n) = target {
            // `c++` on a `const` is an assignment and throws like one. The
            // numeric fast path below writes the slot directly, and the general
            // path reaches the check through `compile_bind`, so this has to come
            // before both — otherwise `const c = 1; c++` silently incremented a
            // constant while `c = 2` correctly threw.
            if self.slots.consts.contains(n) {
                self.throw_const_assignment(b);
                return Ok(());
            }
            if let Some(slot) = self.numeric_slot_of(n) {
                b.emit(Op::GetSlot(slot), 0); // [old]
                if !prefix {
                    b.emit(Op::Dup, 0); // [old, old]
                }
                b.emit(Op::LoadFloat(tag as f64), 0);
                b.emit(Op::Add, 0); // [ (old,) new ]
                if prefix {
                    b.emit(Op::Dup, 0); // [new, new]
                }
                b.emit(Op::SetSlot(slot), 0); // stores, leaves the yielded value
                return Ok(());
            }
        }
        b.emit(Op::LoadInt(tag), 0);
        self.compile_expr(b, target)?; // [tag, old]
        b.emit(Op::CallBuiltin(ops::NUM_STEP, 2), 0); // [oldN, new]
        if prefix {
            // ++x: discard oldN, store new, yield new.
            b.emit(Op::Swap, 0); // [new, oldN]
            b.emit(Op::Pop, 0); // [new]
            b.emit(Op::Dup, 0); // [new, new]
            self.compile_bind(b, target, BindMode::Assign)?; // stores new -> [new]
        } else {
            // x++: store new, yield oldN.
            self.compile_bind(b, target, BindMode::Assign)?; // stores new -> [oldN]
        }
        Ok(())
    }

    fn compile_member(
        &mut self,
        b: &mut ChunkBuilder,
        object: &Expr,
        property: &str,
        optional: bool,
    ) -> Result<(), String> {
        // `super.prop` — read a data/accessor property off the parent prototype.
        if matches!(object, Expr::Super) {
            self.name_const(b, property);
            b.emit(Op::CallBuiltin(ops::SUPER_GET, 1), 0);
            return Ok(());
        }
        self.compile_expr(b, object)?;
        if optional {
            let jshort = self.emit_optional_guard(b);
            self.name_const(b, property);
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
            // Inside a chain the jump belongs to the chain's end, not to this
            // link's — otherwise the rest of the chain runs on the `undefined`
            // the short-circuit just produced.
            match self.opt_chain.last_mut() {
                Some(frame) => frame.push(jshort),
                None => {
                    let end = b.current_pos();
                    b.patch_jump(jshort, end);
                }
            }
        } else {
            self.name_const(b, property);
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
        }
        Ok(())
    }

    fn compile_index(
        &mut self,
        b: &mut ChunkBuilder,
        object: &Expr,
        index: &Expr,
        optional: bool,
    ) -> Result<(), String> {
        // `super[expr]` READ — the computed twin of `super.prop`, which
        // `compile_member` handles. Without it `super` compiled as a value and
        // the read went against `undefined`.
        if matches!(object, Expr::Super) {
            self.compile_expr(b, index)?;
            b.emit(Op::CallBuiltin(ops::SUPER_GET, 1), 0);
            return Ok(());
        }
        self.compile_expr(b, object)?;
        if optional {
            let jshort = self.emit_optional_guard(b);
            self.compile_off_spine(b, index)?;
            b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0);
            match self.opt_chain.last_mut() {
                Some(frame) => frame.push(jshort),
                None => {
                    let end = b.current_pos();
                    b.patch_jump(jshort, end);
                }
            }
        } else {
            self.compile_off_spine(b, index)?;
            b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0);
        }
        Ok(())
    }

    /// For an optional access: object on TOS. If nullish, replace with undefined
    /// and jump over the access. Returns the jump index to patch to the end.
    // ── block scopes ─────────────────────────────────────────────────────
    /// Enter a block scope: `let`/`const` declared after this point die at the
    /// matching [`Self::emit_pop_scope`].
    fn emit_push_scope(&mut self, b: &mut ChunkBuilder) {
        b.emit(Op::CallBuiltin(ops::PUSH_SCOPE, 0), 0);
        b.emit(Op::Pop, 0);
        self.scope_depth += 1;
    }

    fn emit_pop_scope(&mut self, b: &mut ChunkBuilder) {
        b.emit(Op::CallBuiltin(ops::POP_SCOPE, 0), 0);
        b.emit(Op::Pop, 0);
        self.scope_depth -= 1;
    }

    /// Replace the innermost scope with a copy of its bindings — the per-iteration
    /// environment that makes each `for (let i …)` pass capture its own `i`.
    fn emit_copy_scope(&self, b: &mut ChunkBuilder) {
        b.emit(Op::CallBuiltin(ops::COPY_SCOPE, 0), 0);
        b.emit(Op::Pop, 0);
    }

    /// Close and drop every for-of/for-in iterator between here and `target`
    /// depth. A jump to an OUTER loop abandons the inner loops, and their
    /// iterators are parked on the VM stack, so they must be popped (running a
    /// generator's `finally` / the iterator protocol's `.return()`) or the outer
    /// `FORITER` would read the wrong stack slot.
    /// Close every iterator this chunk has parked, with a value already on top
    /// of the stack that must survive: the iterators sit UNDER it, so each one
    /// is swapped up, closed, and its result dropped.
    /// Build the chunk being emitted and hand the host its call-site table. Every
    /// chunk goes through here so a site is registered exactly once, under the
    /// `op_hash` `build()` computes.
    fn finish_chunk(&mut self, b: ChunkBuilder) -> Chunk {
        let sites = std::mem::take(&mut self.call_sites);
        let yields = std::mem::take(&mut self.yield_sites);
        let chunk = b.build();
        crate::host::register_call_sites(chunk.op_hash, sites);
        crate::host::register_yield_sites(chunk.op_hash, yields);
        chunk
    }

    /// Record the callee's source text for the call op just emitted at `at`, so
    /// a `TypeError` raised there can name the callee the way V8 does. Nothing
    /// is recorded for a shape `callee_text` declines to print.
    fn note_call_site(&mut self, at: usize, callee: &Expr) {
        if let Some(text) = callee_text(callee) {
            self.call_sites.push((at, text));
        }
    }

    fn emit_close_iters_under_value(&self, b: &mut ChunkBuilder) {
        for _ in 0..self.iter_depth {
            b.emit(Op::Swap, 0); // [.., iter, val] -> [.., val, iter]
            b.emit(Op::CallBuiltin(ops::ITER_CLOSE, 1), 0); // -> [.., val, result]
            b.emit(Op::Pop, 0); // -> [.., val]
        }
    }

    fn emit_close_iters(&self, b: &mut ChunkBuilder, target: usize) {
        for _ in target..self.iter_depth {
            b.emit(Op::CallBuiltin(ops::ITER_CLOSE, 1), 0);
            b.emit(Op::Pop, 0);
        }
    }

    /// Close every block scope between here and `target` depth, without changing
    /// the compile-time depth (the jump that follows leaves this code path).
    fn emit_unwind_scopes(&self, b: &mut ChunkBuilder, target: usize) {
        for _ in target..self.scope_depth {
            b.emit(Op::CallBuiltin(ops::POP_SCOPE, 0), 0);
            b.emit(Op::Pop, 0);
        }
    }

    /// Raise a `break`/`continue` whose target loop is outside this chunk.
    fn emit_signal_jump(&mut self, b: &mut ChunkBuilder, op: u16, label: Option<&str>, line: u32) {
        self.name_const(b, label.unwrap_or(""));
        b.emit(Op::CallBuiltin(op, 1), line);
        b.emit(Op::Pop, line);
        self.chunk_signals = true;
    }

    /// Emit the `SIG_UNWIND` dispatch that runs right after a `TRY` (or after a
    /// loop that may still hold a signal for an outer labeled loop): route a
    /// pending `break`/`continue` to the enclosing loop's exit/continue target, or
    /// halt the chunk so a `return` (or a signal for a loop further out) keeps
    /// propagating.
    fn emit_signal_dispatch(&mut self, b: &mut ChunkBuilder) {
        // `break` lands on the innermost enclosing context, `continue` on the
        // innermost one that CATCHES it — a `switch` catches `break` but not
        // `continue`, so the two targets are resolved INDEPENDENTLY. Either may be
        // absent from this chunk, in which case a signal of that kind keeps
        // travelling outward. (`cont` implies `brk`: a continue-catching loop is
        // itself breakable, so it can never sit above the innermost context.)
        let brk = self
            .loops
            .len()
            .checked_sub(1)
            .filter(|i| *i >= self.chunk_loop_base);
        let cont = self
            .loops
            .iter()
            .rposition(|c| c.catches_continue)
            .filter(|i| *i >= self.chunk_loop_base);
        let tag_of = |i: Option<usize>, loops: &[LoopCtx]| match i {
            Some(i) => loops[i]
                .label
                .clone()
                .unwrap_or_else(|| unwind::PLAIN_LOOP.to_string()),
            None => unwind::NO_LOOP.to_string(),
        };
        let brk_tag = tag_of(brk, &self.loops);
        let cont_tag = tag_of(cont, &self.loops);
        self.name_const(b, &brk_tag);
        self.name_const(b, &cont_tag);
        b.emit(Op::CallBuiltin(ops::SIG_UNWIND, 2), 0); // [code]
        let Some(idx) = brk else {
            // Nothing in this chunk can catch the signal; `SIG_UNWIND` already
            // halted the chunk, so just drop its code.
            b.emit(Op::Pop, 0);
            return;
        };
        b.emit(Op::Dup, 0);
        b.emit(Op::LoadInt(unwind::BREAK), 0);
        b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
        let jb = b.emit(Op::JumpIfTrue(0), 0);
        let jc = cont.map(|_| {
            b.emit(Op::Dup, 0);
            b.emit(Op::LoadInt(unwind::CONTINUE), 0);
            b.emit(Op::CallBuiltin(ops::STRICT_EQ, 2), 0);
            b.emit(Op::JumpIfTrue(0), 0)
        });
        b.emit(Op::Pop, 0); // no signal: drop the code and fall through
        let jafter = b.emit(Op::Jump(0), 0);
        // The landing pads leave every block scope and iterator opened between
        // here and the target, exactly as the plain compiler-resolved `break` /
        // `continue` does. Skipping this leaked a scope onto the frame, so the
        // NEXT `let`/`const` at that level bound in a dead child env and became
        // invisible to any closure created afterwards.
        let (brk_scope, brk_iter) = (self.loops[idx].break_depth, self.loops[idx].iter_depth);
        let brk_land = b.current_pos();
        b.emit(Op::Pop, 0);
        self.emit_unwind_scopes(b, brk_scope);
        self.emit_close_iters(b, brk_iter);
        let brk_jump = b.emit(Op::Jump(0), 0);
        let cont_jump = jc.map(|_| {
            let (cs, ci) = cont
                .map(|i| (self.loops[i].continue_depth, self.loops[i].iter_depth))
                .unwrap_or((self.scope_depth, self.iter_depth));
            let cont_land = b.current_pos();
            b.emit(Op::Pop, 0);
            self.emit_unwind_scopes(b, cs);
            self.emit_close_iters(b, ci);
            (cont_land, b.emit(Op::Jump(0), 0))
        });
        let after = b.current_pos();
        b.patch_jump(jb, brk_land);
        if let (Some(jc), Some((cont_land, _))) = (jc, cont_jump) {
            b.patch_jump(jc, cont_land);
        }
        b.patch_jump(jafter, after);
        self.loops[idx].breaks.push(brk_jump);
        if let (Some(cont_idx), Some((_, cj))) = (cont, cont_jump) {
            self.loops[cont_idx].continues.push(cj);
        }
    }

    /// Whether `e` is a link in an optional chain that short-circuits — i.e.
    /// walking the SPINE (a member's object, an index's object, a call's
    /// callee) reaches a `?.`. An argument or a computed index is not on the
    /// spine: `a?.b[c?.d]` is two chains, not one.
    fn spine_has_optional(e: &Expr) -> bool {
        match e {
            Expr::Member {
                object, optional, ..
            } => *optional || Self::spine_has_optional(object),
            Expr::Index {
                object, optional, ..
            } => *optional || Self::spine_has_optional(object),
            Expr::Call { func, optional, .. } => *optional || Self::spine_has_optional(func),
            _ => false,
        }
    }

    /// Lower `e` as the ROOT of an optional chain: every `?.` inside its spine
    /// parks a jump, and all of them land here, past the whole chain.
    fn compile_chain_root(&mut self, b: &mut ChunkBuilder, e: &Expr) -> Result<(), String> {
        self.opt_chain.push(Vec::new());
        let r = self.compile_expr(b, e);
        let pending = self.opt_chain.pop().unwrap_or_default();
        r?;
        let end = b.current_pos();
        for j in pending {
            b.patch_jump(j, end);
        }
        Ok(())
    }

    /// Lower `e` with the enclosing chain SUSPENDED, so a `?.` inside it forms
    /// its own chain. Used for the parts that are not on the spine — call
    /// arguments and a computed index.
    fn compile_off_spine(&mut self, b: &mut ChunkBuilder, e: &Expr) -> Result<(), String> {
        let saved = std::mem::take(&mut self.opt_chain);
        let r = self.compile_expr(b, e);
        self.opt_chain = saved;
        r
    }

    fn emit_optional_guard(&mut self, b: &mut ChunkBuilder) -> usize {
        b.emit(Op::Dup, 0);
        b.emit(Op::CallBuiltin(ops::NULLISH, 1), 0);
        let jnull = b.emit(Op::JumpIfFalse(0), 0); // not nullish -> continue access
                                                   // nullish: drop object, push undefined, jump to end.
        b.emit(Op::Pop, 0);
        b.emit(Op::LoadUndef, 0);
        let jend = b.emit(Op::Jump(0), 0);
        let cont = b.current_pos();
        b.patch_jump(jnull, cont);
        jend
    }

    /// `callee?.(args)` — the CALLEE itself may be nullish, in which case the whole
    /// call short-circuits to `undefined` without evaluating the arguments. A
    /// method callee (`obj.m?.()`) must still be invoked with `this === obj`, so it
    /// is dispatched through `m.call(obj, …)` / `m.apply(obj, …)`.
    fn compile_optional_call(
        &mut self,
        b: &mut ChunkBuilder,
        func: &Expr,
        args: &[Expr],
    ) -> Result<(), String> {
        let has_spread = args.iter().any(|a| matches!(a, Expr::Spread(_)));
        if let Expr::Member {
            object,
            property,
            optional: obj_optional,
        } = func
        {
            self.compile_expr(b, object)?; // [recv]
            let jobj = if *obj_optional {
                Some(self.emit_optional_guard(b))
            } else {
                None
            };
            b.emit(Op::Dup, 0); // [recv, recv]
            self.name_const(b, property); // [recv, recv, name]
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0); // [recv, fn]
                                                         // Nullish callee: drop both the method and the receiver.
            b.emit(Op::Dup, 0);
            b.emit(Op::CallBuiltin(ops::NULLISH, 1), 0);
            let jlive = b.emit(Op::JumpIfFalse(0), 0);
            b.emit(Op::Pop, 0);
            b.emit(Op::Pop, 0);
            b.emit(Op::LoadUndef, 0);
            let jend = b.emit(Op::Jump(0), 0);
            let live = b.current_pos();
            b.patch_jump(jlive, live);
            // [recv, fn] -> fn.call(recv, …) / fn.apply(recv, argsArray)
            let via = if has_spread { "apply" } else { "call" };
            self.name_const(b, via); // [recv, fn, via]
            b.emit(Op::Rot, 0); // [fn, via, recv]
            let extra = if has_spread {
                self.compile_spread_args(b, args)?; // [fn, via, recv, argsArray]
                1
            } else {
                for a in args {
                    self.compile_expr(b, a)?;
                }
                args.len()
            };
            b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(3 + extra)?), 0);
            match self.opt_chain.last_mut() {
                Some(frame) => {
                    frame.push(jend);
                    if let Some(j) = jobj {
                        frame.push(j);
                    }
                }
                None => {
                    let end = b.current_pos();
                    b.patch_jump(jend, end);
                    if let Some(j) = jobj {
                        b.patch_jump(j, end);
                    }
                }
            }
            return Ok(());
        }
        // `recv[expr]?.(…)` — the optional-call form of a COMPUTED member. Same
        // receiver rule as `recv.name?.(…)` above; only the key differs, being
        // known at run time rather than compile time. This used to fall through
        // to the plain-callee path below and lose `this`, so `o['self']?.()`
        // threw where `o.self?.()` worked.
        if let Expr::Index {
            object,
            index,
            optional: obj_optional,
        } = func
        {
            self.compile_expr(b, object)?; // [recv]
            let jobj = if *obj_optional {
                Some(self.emit_optional_guard(b))
            } else {
                None
            };
            b.emit(Op::Dup, 0); // [recv, recv]
            self.compile_expr(b, index)?; // [recv, recv, key]
            b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0); // [recv, fn]
            b.emit(Op::Dup, 0);
            b.emit(Op::CallBuiltin(ops::NULLISH, 1), 0);
            let jlive = b.emit(Op::JumpIfFalse(0), 0);
            b.emit(Op::Pop, 0);
            b.emit(Op::Pop, 0);
            b.emit(Op::LoadUndef, 0);
            let jend = b.emit(Op::Jump(0), 0);
            let live = b.current_pos();
            b.patch_jump(jlive, live);
            let via = if has_spread { "apply" } else { "call" };
            self.name_const(b, via); // [recv, fn, via]
            b.emit(Op::Rot, 0); // [fn, via, recv]
            let extra = if has_spread {
                self.compile_spread_args(b, args)?;
                1
            } else {
                for a in args {
                    self.compile_expr(b, a)?;
                }
                args.len()
            };
            b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(3 + extra)?), 0);
            match self.opt_chain.last_mut() {
                Some(frame) => {
                    frame.push(jend);
                    if let Some(j) = jobj {
                        frame.push(j);
                    }
                }
                None => {
                    let end = b.current_pos();
                    b.patch_jump(jend, end);
                    if let Some(j) = jobj {
                        b.patch_jump(j, end);
                    }
                }
            }
            return Ok(());
        }
        // Plain callee (`f?.()`): evaluate it, guard, then call with no
        // receiver — a bare expression has none to keep.
        self.compile_expr(b, func)?;
        let jend = self.emit_optional_guard(b);
        if has_spread {
            self.compile_spread_args(b, args)?;
            b.emit(Op::CallBuiltin(ops::APPLY, 2), 0);
        } else {
            for a in args {
                self.compile_expr(b, a)?;
            }
            b.emit(Op::CallBuiltin(ops::CALL_VALUE, argc(1 + args.len())?), 0);
        }
        let end = b.current_pos();
        b.patch_jump(jend, end);
        Ok(())
    }

    fn compile_call(
        &mut self,
        b: &mut ChunkBuilder,
        func: &Expr,
        args: &[Expr],
        optional: bool,
    ) -> Result<(), String> {
        if optional {
            return self.compile_optional_call(b, func, args);
        }
        let has_spread = args.iter().any(|a| matches!(a, Expr::Spread(_)));
        match func {
            // `super(...args)` — invoke the parent constructor on the current
            // `this` (SUPER_CALL runs the parent ctor + this class's field inits).
            Expr::Super => {
                for a in args {
                    self.compile_expr(b, a)?;
                }
                b.emit(Op::CallBuiltin(ops::SUPER_CALL, argc(args.len())?), 0);
                return Ok(());
            }
            // `super.method(...args)` — resolve the parent method, call it bound to
            // the current `this` via `method.call(this, ...args)`.
            Expr::Member {
                object, property, ..
            } if matches!(**object, Expr::Super) => {
                self.name_const(b, property);
                b.emit(Op::CallBuiltin(ops::SUPER_GET, 1), 0); // [method]
                self.name_const(b, "call"); // [method, "call"]
                b.emit(Op::CallBuiltin(ops::THIS, 0), 0); // [method, "call", this]
                                                          // `method.call(this, ...args)`: compile args (spread expands into
                                                          // the flat run) and dispatch as a method call named "call".
                for a in args {
                    self.compile_expr(b, a)?;
                }
                b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(3 + args.len())?), 0);
                return Ok(());
            }
            Expr::Member {
                object,
                property,
                optional,
            } => {
                self.compile_expr(b, object)?;
                // `obj?.method(...)`: if `obj` is nullish, short-circuit the whole
                // call to `undefined` (skip the method name, args, and dispatch).
                let jshort = if *optional {
                    Some(self.emit_optional_guard(b))
                } else {
                    None
                };
                self.name_const(b, property);
                if has_spread {
                    self.compile_spread_args(b, args)?; // [recv, name, argsArray]
                    b.emit(Op::CallBuiltin(ops::APPLY_METHOD, 3), 0);
                } else {
                    // Arguments are not on the chain's spine: a `?.` inside one
                    // is its own chain and must not jump past this call.
                    for a in args {
                        self.compile_off_spine(b, a)?;
                    }
                    let at = b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(2 + args.len())?), 0);
                    self.note_call_site(at, func);
                }
                if let Some(j) = jshort {
                    match self.opt_chain.last_mut() {
                        Some(frame) => frame.push(j),
                        None => {
                            let end = b.current_pos();
                            b.patch_jump(j, end);
                        }
                    }
                }
            }
            // `super[expr](args)` — the computed twin of `super.m(args)` above.
            // The dotted form was handled and this was not, so it fell through
            // to the ordinary computed-call path, which compiled `super` as a
            // value and dispatched on that.
            Expr::Index { object, index, .. } if matches!(**object, Expr::Super) => {
                self.compile_expr(b, index)?; // [name]
                b.emit(Op::CallBuiltin(ops::SUPER_GET, 1), 0); // [method]
                self.name_const(b, "call"); // [method, "call"]
                b.emit(Op::CallBuiltin(ops::THIS, 0), 0); // [method, "call", this]
                for a in args {
                    self.compile_expr(b, a)?;
                }
                b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(3 + args.len())?), 0);
                return Ok(());
            }
            Expr::Index {
                object,
                index,
                optional,
            } => {
                // recv[expr](args) — evaluate as a method via computed name.
                self.compile_expr(b, object)?; // [recv]
                                               // `recv?.[expr](...)`: short-circuit to `undefined` when nullish.
                let jshort = if *optional {
                    Some(self.emit_optional_guard(b))
                } else {
                    None
                };
                // 13.3.6 EvaluateCall: the receiver of `recv[expr](...)` is
                // `recv`, exactly as for `recv.name(...)`. This used to read the
                // function with GETITEM, DROP the receiver, and call the value
                // with no `this` — the comment called it "approximated", and it
                // silently produced wrong answers rather than errors:
                //
                //     const o = {x: 42, f() { return this.x }};
                //     o.f()      // 42
                //     o['f']()   // undefined      <- was
                //     c['m']()   // TypeError      <- on a class instance
                //
                // CALL_METHOD/APPLY_METHOD take the name off the STACK, so a
                // computed key dispatches through the same path a static one
                // does and keeps the receiver.
                self.compile_expr(b, index)?; // [recv, name]
                if has_spread {
                    self.compile_spread_args(b, args)?; // [recv, name, argsArray]
                    b.emit(Op::CallBuiltin(ops::APPLY_METHOD, 3), 0);
                } else {
                    for a in args {
                        self.compile_off_spine(b, a)?;
                    }
                    let at = b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(2 + args.len())?), 0);
                    self.note_call_site(at, func);
                }
                if let Some(j) = jshort {
                    let end = b.current_pos();
                    b.patch_jump(j, end);
                }
            }
            // A slotted callee has no name to resolve at run time: it falls
            // through to the value path below, which reads the slot and calls
            // through `CALL_VALUE`.
            Expr::Ident(n) if self.slot_of(n).is_none() => {
                self.name_const(b, n);
                if has_spread {
                    self.compile_spread_args(b, args)?; // [name, argsArray]
                                                        // Resolve name to a value, then APPLY.
                    b.emit(Op::Swap, 0); // [argsArray, name]
                    b.emit(Op::CallBuiltin(ops::GETLOCAL, 1), 0); // [argsArray, fn]
                    b.emit(Op::Swap, 0); // [fn, argsArray]
                    b.emit(Op::CallBuiltin(ops::APPLY, 2), 0);
                } else {
                    for a in args {
                        self.compile_expr(b, a)?;
                    }
                    let at = b.emit(Op::CallBuiltin(ops::CALL, argc(1 + args.len())?), 0);
                    self.note_call_site(at, func);
                }
            }
            _ => {
                self.compile_expr(b, func)?;
                if has_spread {
                    self.compile_spread_args(b, args)?;
                    b.emit(Op::CallBuiltin(ops::APPLY, 2), 0);
                } else {
                    for a in args {
                        self.compile_expr(b, a)?;
                    }
                    let at = b.emit(Op::CallBuiltin(ops::CALL_VALUE, argc(1 + args.len())?), 0);
                    self.note_call_site(at, func);
                }
            }
        }
        Ok(())
    }

    /// Build a flat args array from a mix of plain args and `...spread` args.
    fn compile_spread_args(&mut self, b: &mut ChunkBuilder, args: &[Expr]) -> Result<(), String> {
        for a in args {
            match a {
                Expr::Spread(inner) => {
                    b.emit(Op::LoadInt(1), 0);
                    self.compile_expr(b, inner)?;
                }
                _ => {
                    b.emit(Op::LoadInt(0), 0);
                    self.compile_expr(b, a)?;
                }
            }
        }
        b.emit(Op::CallBuiltin(ops::BUILD_ARGS, argc(args.len() * 2)?), 0);
        Ok(())
    }

    fn compile_new(
        &mut self,
        b: &mut ChunkBuilder,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<(), String> {
        self.compile_expr(b, callee)?;
        for a in args {
            self.compile_expr(b, a)?;
        }
        let at = b.emit(Op::CallBuiltin(ops::NEW, argc(1 + args.len())?), 0);
        self.note_call_site(at, callee);
        Ok(())
    }
}

/// A prologue statement applying a parameter default: `if (name === undefined)
/// name = default;`.
fn default_stmt(name: &str, default: &Expr) -> Stmt {
    Stmt::from(StmtKind::If {
        test: Expr::Binary(
            BinOp::EqEqEq,
            Box::new(Expr::Ident(name.to_string())),
            Box::new(Expr::Undefined),
        ),
        cons: Box::new(Stmt::from(StmtKind::Expr(Expr::Assign {
            target: Box::new(Expr::Ident(name.to_string())),
            value: Box::new(default.clone()),
        }))),
        alt: None,
    })
}

/// Every name a `var` binds inside one function scope, in source order.
///
/// Descends through block-scoped constructs, because `var` is not block-scoped,
/// and stops at a nested `function` declaration, whose body is its own scope.
/// Function *expressions* and arrows are inside `Expr`, which is not walked at
/// all: a `var` can only be introduced by a statement.
fn collect_var_names(s: &Stmt, out: &mut Vec<String>) {
    match &s.kind {
        StmtKind::Decl {
            kind: DeclKind::Var,
            decls,
        } => {
            for d in decls {
                pattern_names(&d.target, out);
            }
        }
        StmtKind::Block(body) => body.iter().for_each(|s| collect_var_names(s, out)),
        StmtKind::If { cons, alt, .. } => {
            collect_var_names(cons, out);
            if let Some(a) = alt {
                collect_var_names(a, out);
            }
        }
        StmtKind::While { body, .. }
        | StmtKind::DoWhile { body, .. }
        | StmtKind::Labeled { body, .. } => collect_var_names(body, out),
        StmtKind::For { init, body, .. } => {
            if let Some(i) = init {
                collect_var_names(i, out);
            }
            collect_var_names(body, out);
        }
        StmtKind::ForOf {
            decl_kind,
            target,
            body,
            ..
        }
        | StmtKind::ForIn {
            decl_kind,
            target,
            body,
            ..
        } => {
            if *decl_kind == Some(DeclKind::Var) {
                pattern_names(target, out);
            }
            collect_var_names(body, out);
        }
        StmtKind::Switch { cases, .. } => {
            for c in cases {
                c.body.iter().for_each(|s| collect_var_names(s, out));
            }
        }
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            block.iter().for_each(|s| collect_var_names(s, out));
            if let Some((_, body)) = handler {
                // The catch PARAMETER is block-scoped to the handler, so it is
                // not collected; a `var` in the handler body still hoists.
                body.iter().for_each(|s| collect_var_names(s, out));
            }
            if let Some(f) = finalizer {
                f.iter().for_each(|s| collect_var_names(s, out));
            }
        }
        _ => {}
    }
}

/// The binding names a declaration target introduces, destructuring included.
fn pattern_names(target: &Expr, out: &mut Vec<String>) {
    match target {
        Expr::Ident(n) => {
            if !out.iter().any(|x| x == n) {
                out.push(n.clone());
            }
        }
        Expr::Array(items) => items.iter().for_each(|i| pattern_names(i, out)),
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::KeyValue { value, .. } => pattern_names(value, out),
                    Prop::Spread(e) => pattern_names(e, out),
                    Prop::Accessor { .. } => {}
                }
            }
        }
        // `[a = 1]` / `{a: b = 1}` — the binding is the target, not the default.
        Expr::Assign { target, .. } => pattern_names(target, out),
        Expr::Spread(inner) => pattern_names(inner, out),
        // A member target (`[obj.x] = …`) assigns a property, binding nothing.
        _ => {}
    }
}
