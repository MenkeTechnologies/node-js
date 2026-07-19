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
use crate::host::{binop as bop, member, ops, unop, FuncDef, ParamSlot, TryDef};
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

/// Break/continue jump fixups for a loop or switch.
struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    /// Whether `continue` binds here (true for loops, false for `switch`).
    catches_continue: bool,
}

#[derive(Default)]
pub struct Compiler {
    functions: Vec<(String, FuncDef)>,
    tries: Vec<TryDef>,
    loops: Vec<LoopCtx>,
    tmp: usize,
    /// Emit per-statement `DBG_LINE` markers for the DAP debugger (`node --dap`).
    debug: bool,
}

/// Compile a parsed program. `debug` enables per-statement DAP line markers.
pub fn compile(stmts: &[Stmt], debug: bool) -> Result<Program, String> {
    let mut c = Compiler {
        debug,
        ..Default::default()
    };
    let mut b = ChunkBuilder::new();
    // Hoist function declarations to the top (JS function hoisting).
    c.hoist_funcs(&mut b, stmts)?;
    c.compile_stmts(&mut b, stmts)?;
    Ok(Program {
        main: b.build(),
        functions: c.functions,
        tries: c.tries,
    })
}

fn argc(n: usize) -> Result<u8, String> {
    u8::try_from(n).map_err(|_| "too many arguments (>255) for one call".to_string())
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
                self.declare(b, &Expr::Ident(name.clone()));
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
            StmtKind::Decl { decls, .. } => {
                for d in decls {
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
                    self.compile_bind(b, &d.target, true)?;
                }
            }
            StmtKind::Block(body) => {
                self.hoist_funcs(b, body)?;
                self.compile_stmts(b, body)?;
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
            } => self.compile_for_of(b, decl_kind.is_some(), target, iter, body)?,
            StmtKind::ForIn {
                decl_kind,
                target,
                object,
                body,
            } => self.compile_for_in(b, decl_kind.is_some(), target, object, body)?,
            StmtKind::Switch { disc, cases } => self.compile_switch(b, disc, cases)?,
            StmtKind::Return(e) => {
                match e {
                    Some(e) => self.compile_expr(b, e)?,
                    None => {
                        b.emit(Op::LoadUndef, line);
                    }
                }
                b.emit(Op::CallBuiltin(ops::SIG_RETURN, 1), line);
            }
            StmtKind::Break(_) => {
                let j = b.emit(Op::Jump(0), line);
                self.loops
                    .last_mut()
                    .ok_or("SyntaxError: 'break' outside loop")?
                    .breaks
                    .push(j);
            }
            StmtKind::Continue(_) => {
                let j = b.emit(Op::Jump(0), line);
                self.loops
                    .iter_mut()
                    .rev()
                    .find(|c| c.catches_continue)
                    .ok_or("SyntaxError: 'continue' outside loop")?
                    .continues
                    .push(j);
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
    fn compile_bind(&mut self, b: &mut ChunkBuilder, target: &Expr, declare: bool) -> Result<(), String> {
        match target {
            Expr::Ident(_) => {
                if declare {
                    self.declare(b, target);
                } else {
                    self.store_simple(b, target)?;
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
        if let Expr::Ident(n) = target {
            self.name_const(b, n);
            b.emit(Op::Swap, 0);
            b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
            b.emit(Op::Pop, 0);
        }
    }

    /// Store TOS into an lvalue (Ident/Member/Index), leaving nothing.
    fn store_simple(&mut self, b: &mut ChunkBuilder, target: &Expr) -> Result<(), String> {
        match target {
            Expr::Ident(n) => {
                self.name_const(b, n);
                b.emit(Op::Swap, 0);
                b.emit(Op::CallBuiltin(ops::SETLOCAL, 2), 0);
                b.emit(Op::Pop, 0);
            }
            Expr::Member { object, property, .. } => {
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

    fn destructure_array(&mut self, b: &mut ChunkBuilder, items: &[Expr], declare: bool) -> Result<(), String> {
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
                Expr::Undefined => {
                    b.emit(Op::Pop, 0); // hole
                }
                Expr::Spread(inner) => self.compile_bind(b, inner, declare)?,
                _ => self.compile_bind(b, it, declare)?,
            }
        }
        Ok(())
    }

    fn destructure_object(&mut self, b: &mut ChunkBuilder, props: &[Prop], declare: bool) -> Result<(), String> {
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
        self.name_const(b, name);
        b.emit(Op::CallBuiltin(ops::GETLOCAL, 1), 0);
    }

    // ── control flow ─────────────────────────────────────────────────────
    fn compile_condition(&mut self, b: &mut ChunkBuilder, e: &Expr) -> Result<(), String> {
        self.compile_expr(b, e)?;
        b.emit(Op::CallBuiltin(ops::TRUTHY, 1), 0);
        Ok(())
    }

    fn compile_if(&mut self, b: &mut ChunkBuilder, test: &Expr, cons: &Stmt, alt: &Option<Box<Stmt>>) -> Result<(), String> {
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

    fn compile_while(&mut self, b: &mut ChunkBuilder, test: &Expr, body: &Stmt) -> Result<(), String> {
        let start = b.current_pos();
        self.compile_condition(b, test)?;
        let jfalse = b.emit(Op::JumpIfFalse(0), 0);
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            catches_continue: true,
        });
        self.compile_stmt(b, body)?;
        b.emit(Op::Jump(start), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, start);
        }
        let end = b.current_pos();
        b.patch_jump(jfalse, end);
        for br in ctx.breaks {
            b.patch_jump(br, end);
        }
        Ok(())
    }

    fn compile_do_while(&mut self, b: &mut ChunkBuilder, body: &Stmt, test: &Expr) -> Result<(), String> {
        let start = b.current_pos();
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            catches_continue: true,
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
        if let Some(init) = init {
            self.compile_stmt(b, init)?;
        }
        let start = b.current_pos();
        let jfalse = match test {
            Some(t) => {
                self.compile_condition(b, t)?;
                Some(b.emit(Op::JumpIfFalse(0), 0))
            }
            None => None,
        };
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            catches_continue: true,
        });
        self.compile_stmt(b, body)?;
        let cont_target = b.current_pos();
        if let Some(u) = update {
            self.compile_expr(b, u)?;
            b.emit(Op::Pop, 0);
        }
        b.emit(Op::Jump(start), 0);
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
        Ok(())
    }

    fn compile_for_of(&mut self, b: &mut ChunkBuilder, declare: bool, target: &Expr, iter: &Expr, body: &Stmt) -> Result<(), String> {
        self.compile_expr(b, iter)?;
        b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
        self.loop_over(b, declare, target, body)
    }

    fn compile_for_in(&mut self, b: &mut ChunkBuilder, declare: bool, target: &Expr, object: &Expr, body: &Stmt) -> Result<(), String> {
        self.compile_expr(b, object)?;
        b.emit(Op::CallBuiltin(ops::FORIN_KEYS, 1), 0); // [keys_array]
        b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
        self.loop_over(b, declare, target, body)
    }

    /// Shared loop tail for for-of / for-in: iterator on TOS.
    fn loop_over(&mut self, b: &mut ChunkBuilder, declare: bool, target: &Expr, body: &Stmt) -> Result<(), String> {
        let start = b.current_pos();
        b.emit(Op::CallBuiltin(ops::FORITER, 0), 0); // [iterator, value, has_next]
        let jdone = b.emit(Op::JumpIfFalse(0), 0); // pops has_next
        self.compile_bind(b, target, declare)?; // consumes value -> [iterator]
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            catches_continue: true,
        });
        self.compile_stmt(b, body)?;
        b.emit(Op::Jump(start), 0);
        let ctx = self.loops.pop().unwrap();
        for c in ctx.continues {
            b.patch_jump(c, start);
        }
        let done = b.current_pos();
        b.patch_jump(jdone, done);
        b.emit(Op::Pop, 0); // drop iterator
        let jafter = b.emit(Op::Jump(0), 0);
        let break_target = b.current_pos();
        b.emit(Op::Pop, 0); // drop iterator on break
        let end = b.current_pos();
        b.patch_jump(jafter, end);
        for br in ctx.breaks {
            b.patch_jump(br, break_target);
        }
        Ok(())
    }

    fn compile_switch(&mut self, b: &mut ChunkBuilder, disc: &Expr, cases: &[SwitchCase]) -> Result<(), String> {
        let disc_tmp = self.tmp_name("switch");
        self.compile_expr(b, disc)?;
        self.name_const(b, &disc_tmp);
        b.emit(Op::Swap, 0);
        b.emit(Op::CallBuiltin(ops::DECLARE, 2), 0);
        b.emit(Op::Pop, 0);
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
            catches_continue: false,
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
        Ok(())
    }

    fn compile_block_chunk(&mut self, stmts: &[Stmt]) -> Result<Chunk, String> {
        let mut cb = ChunkBuilder::new();
        self.hoist_funcs(&mut cb, stmts)?;
        self.compile_stmts(&mut cb, stmts)?;
        Ok(cb.build())
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
        let (slots, prologue) = self.lower_params(params)?;
        let mut fb = ChunkBuilder::new();
        // Function-body function hoisting.
        self.hoist_funcs(&mut fb, &prologue)?;
        self.hoist_funcs(&mut fb, body)?;
        self.compile_stmts(&mut fb, &prologue)?;
        self.compile_stmts(&mut fb, body)?;
        let def = FuncDef {
            name: name.to_string(),
            params: slots,
            chunk: fb.build(),
            is_arrow: false,
            is_generator,
            is_async,
        };
        self.functions.push((name.to_string(), def));
        Ok(self.functions.len() - 1)
    }

    fn build_arrow(&mut self, params: &[Param], body: &FnBody, is_async: bool) -> Result<usize, String> {
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
        let cname = node.name.clone().unwrap_or_default();
        // Push name, parent (or undefined), constructor (or undefined).
        self.name_const(b, &cname);
        match &node.parent {
            Some(p) => self.compile_expr(b, p)?,
            None => {
                b.emit(Op::LoadUndef, 0);
            }
        }
        let ctor = node.members.iter().find(|m| m.kind == MemberKind::Constructor);
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

        for m in &node.members {
            match m.kind {
                MemberKind::Constructor => {}
                MemberKind::Field if m.is_static => {
                    // A static field is evaluated once at class-definition time and
                    // set as an own property of the constructor: `[class]` stays on
                    // the stack, `Dup` it as the SETATTR receiver.
                    b.emit(Op::Dup, 0); // [class, class]
                    self.emit_member_key(b, m)?; // [class, class, name]
                    match &m.field_init {
                        Some(e) => self.compile_expr(b, e)?,
                        None => {
                            b.emit(Op::LoadUndef, 0);
                        }
                    }
                    // [class, class, name, val] -> SETATTR sets on the class -> [class, val]
                    b.emit(Op::CallBuiltin(ops::SETATTR, 3), 0);
                    b.emit(Op::Pop, 0); // drop the returned value -> [class]
                }
                MemberKind::Field => {
                    // [class] name thunk -> DEF_FIELD -> [class]
                    self.emit_member_key(b, m)?;
                    let init = m.field_init.clone().unwrap_or(Expr::Undefined);
                    let stmts = vec![Stmt::from(StmtKind::Return(Some(init)))];
                    let def_id = self.build_function("", &[], &stmts, false, false)?;
                    self.emit_mkfunc(b, def_id);
                    b.emit(Op::CallBuiltin(ops::DEF_FIELD, 3), 0);
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
                    b.emit(if m.is_static { Op::LoadTrue } else { Op::LoadFalse }, 0);
                    let mname = match &m.key {
                        Expr::Str(s) if !m.computed => s.clone(),
                        _ => String::new(),
                    };
                    let def_id = self.build_function(&mname, &m.params, &m.body, m.is_generator, m.is_async)?;
                    self.emit_mkfunc(b, def_id);
                    b.emit(Op::CallBuiltin(ops::DEF_MEMBER, 5), 0);
                }
            }
        }
        Ok(())
    }

    /// If `init` is an anonymous function/arrow/class (value already on TOS), set
    /// its `.name` to `name` (JS binding name-inference). No-op otherwise.
    fn infer_name(&mut self, b: &mut ChunkBuilder, init: &Expr, name: &str) {
        let anon = matches!(
            init,
            Expr::Function { name: None, .. } | Expr::Class(_)
        ) && !matches!(init, Expr::Class(node) if node.name.is_some());
        if !anon {
            return;
        }
        // [fn] Dup; .name = name; drop the SETATTR result.
        b.emit(Op::Dup, 0);
        self.name_const(b, "name");
        self.strlit(b, name);
        b.emit(Op::CallBuiltin(ops::SETATTR, 3), 0);
        b.emit(Op::Pop, 0);
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
    fn compile_yield(&mut self, b: &mut ChunkBuilder, arg: &Option<Box<Expr>>, delegate: bool) -> Result<(), String> {
        if delegate {
            // `yield* iterable`: iterate, yielding each element.
            match arg {
                Some(e) => self.compile_expr(b, e)?,
                None => {
                    b.emit(Op::LoadUndef, 0);
                }
            }
            b.emit(Op::CallBuiltin(ops::GETITER, 1), 0); // [iterator]
            let start = b.current_pos();
            b.emit(Op::CallBuiltin(ops::FORITER, 0), 0); // [iterator, value, has_next]
            let jdone = b.emit(Op::JumpIfFalse(0), 0);
            b.emit(Op::CallBuiltin(ops::YIELD, 1), 0); // yield the value -> [iterator, sent]
            b.emit(Op::Pop, 0); // drop the sent value
            b.emit(Op::Jump(start), 0);
            let done = b.current_pos();
            b.patch_jump(jdone, done);
            b.emit(Op::Pop, 0); // drop the iterator
            b.emit(Op::LoadUndef, 0); // `yield*` evaluates to the delegate's return
        } else {
            match arg {
                Some(e) => self.compile_expr(b, e)?,
                None => {
                    b.emit(Op::LoadUndef, 0);
                }
            }
            // YIELD suspends and leaves the value sent by `.next(x)` on the stack.
            b.emit(Op::CallBuiltin(ops::YIELD, 1), 0);
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
            Expr::Str(s) => self.strlit(b, s),
            Expr::Template { quasis, exprs } => self.compile_template(b, quasis, exprs)?,
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
                b.emit(Op::Dup, 0); // assignment yields the value
                self.compile_bind(b, target, false)?;
            }
            Expr::Update { op, prefix, target } => self.compile_update(b, *op, *prefix, target)?,
            Expr::Call { func, args, optional } => self.compile_call(b, func, args, *optional)?,
            Expr::New { callee, args } => self.compile_new(b, callee, args)?,
            Expr::Member { object, property, optional } => {
                self.compile_member(b, object, property, *optional)?
            }
            Expr::Index { object, index, optional } => {
                self.compile_index(b, object, index, *optional)?
            }
            Expr::Function { params, body, is_arrow, name, is_generator, is_async } => {
                let def_id = if *is_arrow {
                    self.build_arrow(params, body, *is_async)?
                } else {
                    let n = name.clone().unwrap_or_default();
                    let stmts = match body {
                        FnBody::Block(b) => b.clone(),
                        FnBody::Expr(e) => vec![Stmt::from(StmtKind::Return(Some((**e).clone())))],
                    };
                    self.build_function(&n, params, &stmts, *is_generator, *is_async)?
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

    fn compile_template(&mut self, b: &mut ChunkBuilder, quasis: &[String], exprs: &[Expr]) -> Result<(), String> {
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

    fn compile_array(&mut self, b: &mut ChunkBuilder, items: &[Expr]) -> Result<(), String> {
        if items.iter().any(|e| matches!(e, Expr::Spread(_))) {
            // (tag, value) pairs; tag 1 = spread.
            for it in items {
                match it {
                    Expr::Spread(inner) => {
                        b.emit(Op::LoadInt(1), 0);
                        self.compile_expr(b, inner)?;
                    }
                    _ => {
                        b.emit(Op::LoadInt(0), 0);
                        self.compile_expr(b, it)?;
                    }
                }
            }
            b.emit(Op::CallBuiltin(ops::BUILD_ARGS, argc(items.len() * 2)?), 0);
        } else {
            for it in items {
                self.compile_expr(b, it)?;
            }
            b.emit(Op::CallBuiltin(ops::MKARR, argc(items.len())?), 0);
        }
        Ok(())
    }

    fn compile_object(&mut self, b: &mut ChunkBuilder, props: &[Prop]) -> Result<(), String> {
        // (tag, key, val) triples for the data/spread props; tag 1 = ...spread.
        // Accessors are installed afterward via DEF_ACCESSOR.
        let data: Vec<&Prop> = props
            .iter()
            .filter(|p| !matches!(p, Prop::Accessor { .. }))
            .collect();
        for p in &data {
            match p {
                Prop::KeyValue { key, value, .. } => {
                    b.emit(Op::LoadInt(0), 0);
                    // Key coerces to a property key (Symbol-aware: a Symbol maps to
                    // its internal `@@…` key rather than a `String()` coercion).
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                    self.compile_expr(b, value)?;
                }
                Prop::Spread(src) => {
                    b.emit(Op::LoadInt(1), 0);
                    self.compile_expr(b, src)?;
                    b.emit(Op::LoadUndef, 0);
                }
                Prop::Accessor { .. } => unreachable!(),
            }
        }
        b.emit(Op::CallBuiltin(ops::MKOBJ, argc(data.len() * 3)?), 0); // [obj]
        // Install any getters/setters, keeping the object on the stack.
        for p in props {
            if let Prop::Accessor { key, computed, is_getter, func } = p {
                if *computed {
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                } else if let Expr::Str(s) = key {
                    self.name_const(b, s);
                } else {
                    self.compile_expr(b, key)?;
                    b.emit(Op::CallBuiltin(ops::PROPKEY, 1), 0);
                }
                b.emit(Op::LoadInt(if *is_getter { member::GET } else { member::SET }), 0);
                self.compile_expr(b, func)?;
                b.emit(Op::CallBuiltin(ops::DEF_ACCESSOR, 4), 0);
            }
        }
        Ok(())
    }

    fn compile_logical(&mut self, b: &mut ChunkBuilder, op: LogicalOp, l: &Expr, r: &Expr) -> Result<(), String> {
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
                self.compile_expr(b, e)?;
                b.emit(Op::CallBuiltin(ops::TYPEOF, 1), 0);
            }
            UnOp::Void => {
                self.compile_expr(b, e)?;
                b.emit(Op::Pop, 0);
                b.emit(Op::LoadUndef, 0);
            }
            UnOp::Delete => match e {
                Expr::Member { object, property, .. } => {
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

    fn compile_binary(&mut self, b: &mut ChunkBuilder, op: BinOp, l: &Expr, r: &Expr) -> Result<(), String> {
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
            BinOp::Pow => native!(Op::Pow),
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
                self.compile_expr(b, l)?;
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

    fn emit_bitwise(&mut self, b: &mut ChunkBuilder, tag: i64, l: &Expr, r: &Expr) -> Result<(), String> {
        b.emit(Op::LoadInt(tag), 0);
        self.compile_expr(b, l)?;
        self.compile_expr(b, r)?;
        b.emit(Op::CallBuiltin(ops::BINOP, 3), 0);
        Ok(())
    }

    fn compile_update(&mut self, b: &mut ChunkBuilder, op: UpdateOp, prefix: bool, target: &Expr) -> Result<(), String> {
        // Desugar to `target = target +/- 1`, yielding the pre/post value.
        let one = Expr::Number(1.0);
        let bin = if matches!(op, UpdateOp::Inc) { BinOp::Add } else { BinOp::Sub };
        if prefix {
            // ++x: compute new, store, yield new.
            let newv = Expr::Binary(bin, Box::new(target.clone()), Box::new(one));
            self.compile_expr(b, &newv)?;
            b.emit(Op::Dup, 0);
            self.compile_bind(b, target, false)?;
        } else {
            // x++: yield old (as number), store new.
            // Push old coerced to number (+old), keep a copy, add 1, store.
            b.emit(Op::LoadInt(unop::POS), 0);
            self.compile_expr(b, target)?;
            b.emit(Op::CallBuiltin(ops::UNARY, 2), 0); // [oldNum]
            b.emit(Op::Dup, 0); // [oldNum, oldNum]
            b.emit(Op::LoadFloat(1.0), 0);
            match op {
                UpdateOp::Inc => b.emit(Op::Add, 0),
                UpdateOp::Dec => b.emit(Op::Sub, 0),
            };
            self.compile_bind(b, target, false)?; // stores new -> [oldNum]
        }
        Ok(())
    }

    fn compile_member(&mut self, b: &mut ChunkBuilder, object: &Expr, property: &str, optional: bool) -> Result<(), String> {
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
            let end = b.current_pos();
            b.patch_jump(jshort, end);
        } else {
            self.name_const(b, property);
            b.emit(Op::CallBuiltin(ops::GETATTR, 2), 0);
        }
        Ok(())
    }

    fn compile_index(&mut self, b: &mut ChunkBuilder, object: &Expr, index: &Expr, optional: bool) -> Result<(), String> {
        self.compile_expr(b, object)?;
        if optional {
            let jshort = self.emit_optional_guard(b);
            self.compile_expr(b, index)?;
            b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0);
            let end = b.current_pos();
            b.patch_jump(jshort, end);
        } else {
            self.compile_expr(b, index)?;
            b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0);
        }
        Ok(())
    }

    /// For an optional access: object on TOS. If nullish, replace with undefined
    /// and jump over the access. Returns the jump index to patch to the end.
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

    fn compile_call(&mut self, b: &mut ChunkBuilder, func: &Expr, args: &[Expr], _optional: bool) -> Result<(), String> {
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
            Expr::Member { object, property, .. } if matches!(**object, Expr::Super) => {
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
            Expr::Member { object, property, .. } => {
                self.compile_expr(b, object)?;
                self.name_const(b, property);
                if has_spread {
                    self.compile_spread_args(b, args)?; // [recv, name, argsArray]
                    b.emit(Op::CallBuiltin(ops::APPLY_METHOD, 3), 0);
                } else {
                    for a in args {
                        self.compile_expr(b, a)?;
                    }
                    b.emit(Op::CallBuiltin(ops::CALL_METHOD, argc(2 + args.len())?), 0);
                }
            }
            Expr::Index { object, index, .. } => {
                // recv[expr](args) — evaluate as a method via computed name.
                self.compile_expr(b, object)?; // [recv]
                b.emit(Op::Dup, 0); // [recv, recv]
                self.compile_expr(b, index)?; // [recv, recv, idx]
                b.emit(Op::CallBuiltin(ops::GETITEM, 2), 0); // [recv, fn]
                b.emit(Op::Swap, 0); // [fn, recv]... but APPLY needs callable then this
                // Fall back: call the function value with `this`=recv via CALL_VALUE
                // (this-binding for computed method calls is approximated).
                b.emit(Op::Pop, 0); // drop recv; keep fn on stack: [fn]
                if has_spread {
                    self.compile_spread_args(b, args)?;
                    b.emit(Op::CallBuiltin(ops::APPLY, 2), 0);
                } else {
                    for a in args {
                        self.compile_expr(b, a)?;
                    }
                    b.emit(Op::CallBuiltin(ops::CALL_VALUE, argc(1 + args.len())?), 0);
                }
            }
            Expr::Ident(n) => {
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
                    b.emit(Op::CallBuiltin(ops::CALL, argc(1 + args.len())?), 0);
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
                    b.emit(Op::CallBuiltin(ops::CALL_VALUE, argc(1 + args.len())?), 0);
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

    fn compile_new(&mut self, b: &mut ChunkBuilder, callee: &Expr, args: &[Expr]) -> Result<(), String> {
        self.compile_expr(b, callee)?;
        for a in args {
            self.compile_expr(b, a)?;
        }
        b.emit(Op::CallBuiltin(ops::NEW, argc(1 + args.len())?), 0);
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
