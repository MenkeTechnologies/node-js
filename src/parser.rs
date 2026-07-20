//! JavaScript parser: token stream → AST.
//!
//! Recursive descent with precedence climbing for binary operators. Automatic
//! Semicolon Insertion is applied at statement boundaries using the
//! `newline_before` flag the lexer records on every token. Arrow functions are
//! detected at assignment level by looking ahead for `=>` after a parameter
//! list. Template-literal `${...}` fields are re-parsed here from the raw source
//! the lexer captured.

use crate::ast::*;
use crate::lexer::{lex, Tok, Token};

const KEYWORDS: &[&str] = &[
    "var", "let", "const", "function", "return", "if", "else", "while", "do", "for", "of", "in",
    "switch", "case", "default", "break", "continue", "true", "false", "null", "this", "new",
    "typeof", "void", "delete", "instanceof", "throw", "try", "catch", "finally",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// True while parsing a generator body — enables `yield` as an operator.
    in_generator: bool,
    /// True while parsing an async body — enables `await` as an operator.
    in_async: bool,
}

/// Parse a complete JS program into a statement list. Inline `rust { ... }` FFI
/// blocks are desugared to `__rust_compile(...)` calls before lexing.
pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let src = crate::rust_ffi::desugar(src);
    let toks = lex(&src)?;
    let mut p = Parser { toks, pos: 0, in_generator: false, in_async: false };
    let mut out = Vec::new();
    while !p.at_eof() {
        out.push(p.parse_stmt()?);
    }
    Ok(out)
}

impl Parser {
    // ── token helpers ────────────────────────────────────────────────────
    fn cur(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn tok(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }
    fn at_eof(&self) -> bool {
        matches!(self.tok(), Tok::Eof)
    }
    fn newline_before(&self) -> bool {
        self.cur().newline_before
    }
    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    /// True if the current token is the punctuation `s`.
    fn is_punct(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Punct(p) if p == s)
    }
    /// True if the current token is the identifier/keyword `s`.
    fn is_kw(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Ident(i) if i == s)
    }
    /// Consume the punctuation `s` if present.
    fn eat_punct(&mut self, s: &str) -> bool {
        if self.is_punct(s) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, s: &str) -> bool {
        if self.is_kw(s) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expect_punct(&mut self, s: &str) -> Result<(), String> {
        if self.eat_punct(s) {
            Ok(())
        } else {
            Err(format!(
                "SyntaxError: expected '{s}' but found {:?} (line {})",
                self.tok(),
                self.line()
            ))
        }
    }

    /// Consume an identifier name (any non-punct ident, including keywords used
    /// as property names when `allow_kw`).
    fn ident_name(&mut self) -> Result<String, String> {
        match self.tok().clone() {
            Tok::Ident(s) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!(
                "SyntaxError: expected identifier but found {other:?} (line {})",
                self.line()
            )),
        }
    }

    /// Apply ASI: consume an explicit `;`, or accept a newline / `}` / EOF.
    fn semicolon(&mut self) -> Result<(), String> {
        if self.eat_punct(";") {
            return Ok(());
        }
        if self.newline_before() || self.is_punct("}") || self.at_eof() {
            return Ok(());
        }
        Err(format!(
            "SyntaxError: expected ';' but found {:?} (line {})",
            self.tok(),
            self.line()
        ))
    }

    // ── statements ───────────────────────────────────────────────────────
    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        let kind = match self.tok().clone() {
            Tok::Punct(p) if p == "{" => {
                self.advance();
                StmtKind::Block(self.parse_block_body()?)
            }
            Tok::Punct(p) if p == ";" => {
                self.advance();
                StmtKind::Empty
            }
            Tok::Ident(kw) if kw == "var" || kw == "let" || kw == "const" => {
                let k = self.parse_decl_kind();
                let decls = self.parse_declarators()?;
                self.semicolon()?;
                StmtKind::Decl { kind: k, decls }
            }
            Tok::Ident(kw) if kw == "function" => self.parse_func_decl(false)?,
            // `async function …` (declaration). `async` stays a plain identifier
            // anywhere else (contextual keyword).
            Tok::Ident(kw) if kw == "async" && self.peek_kw(1, "function") && !self.peek_newline(1) => {
                self.advance(); // async
                self.parse_func_decl(true)?
            }
            Tok::Ident(kw) if kw == "class" => {
                let node = self.parse_class(true)?;
                StmtKind::ClassDecl(node)
            }
            Tok::Ident(kw) if kw == "if" => self.parse_if()?,
            Tok::Ident(kw) if kw == "while" => self.parse_while()?,
            Tok::Ident(kw) if kw == "do" => self.parse_do_while()?,
            Tok::Ident(kw) if kw == "for" => self.parse_for()?,
            Tok::Ident(kw) if kw == "switch" => self.parse_switch()?,
            Tok::Ident(kw) if kw == "return" => {
                self.advance();
                let arg = if self.is_punct(";") || self.is_punct("}") || self.newline_before() || self.at_eof() {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.semicolon()?;
                StmtKind::Return(arg)
            }
            Tok::Ident(kw) if kw == "break" => {
                self.advance();
                let label = self.opt_label();
                self.semicolon()?;
                StmtKind::Break(label)
            }
            Tok::Ident(kw) if kw == "continue" => {
                self.advance();
                let label = self.opt_label();
                self.semicolon()?;
                StmtKind::Continue(label)
            }
            Tok::Ident(kw) if kw == "throw" => {
                self.advance();
                let e = self.parse_expr()?;
                self.semicolon()?;
                StmtKind::Throw(e)
            }
            Tok::Ident(kw) if kw == "try" => self.parse_try()?,
            _ => {
                let e = self.parse_expr()?;
                self.semicolon()?;
                StmtKind::Expr(e)
            }
        };
        Ok(Stmt::new(kind, line))
    }

    /// Whether the token `n` ahead is the identifier `kw`.
    fn peek_kw(&self, n: usize, kw: &str) -> bool {
        matches!(self.toks.get(self.pos + n).map(|t| &t.tok), Some(Tok::Ident(s)) if s == kw)
    }
    /// Whether the token `n` ahead has a newline before it.
    fn peek_newline(&self, n: usize) -> bool {
        self.toks.get(self.pos + n).map(|t| t.newline_before).unwrap_or(false)
    }

    /// Parse a `function` declaration (the `function`/`async function` keyword is
    /// current). `is_async` is true when a preceding `async` was consumed.
    fn parse_func_decl(&mut self, is_async: bool) -> Result<StmtKind, String> {
        self.advance(); // function
        let is_generator = self.eat_punct("*");
        let name = self.ident_name()?;
        let params = self.parse_params()?;
        self.expect_punct("{")?;
        let body = self.parse_fn_body_block(is_generator, is_async)?;
        Ok(StmtKind::FuncDecl {
            name,
            params,
            body,
            is_generator,
            is_async,
        })
    }

    /// Parse a brace-delimited function body under the given generator/async
    /// context (so `yield`/`await` inside are operators, not identifiers).
    fn parse_fn_body_block(&mut self, is_generator: bool, is_async: bool) -> Result<Vec<Stmt>, String> {
        let (pg, pa) = (self.in_generator, self.in_async);
        self.in_generator = is_generator;
        self.in_async = is_async;
        let body = self.parse_block_body();
        self.in_generator = pg;
        self.in_async = pa;
        body
    }

    /// Parse a function *expression* (`function`/`async function`, keyword
    /// current). Supports `function*` generators.
    fn parse_function_expr(&mut self, is_async: bool) -> Result<Expr, String> {
        self.advance(); // function
        let is_generator = self.eat_punct("*");
        let name = if let Tok::Ident(n) = self.tok() {
            if !is_keyword(n) {
                let n = n.clone();
                self.advance();
                Some(n)
            } else {
                None
            }
        } else {
            None
        };
        let params = self.parse_params()?;
        self.expect_punct("{")?;
        let body = self.parse_fn_body_block(is_generator, is_async)?;
        Ok(Expr::Function {
            params,
            body: FnBody::Block(body),
            is_arrow: false,
            name,
            is_generator,
            is_async,
        })
    }

    /// Parse a `class` (the `class` keyword is current). `_decl` distinguishes a
    /// declaration (name required in strict mode, but we accept optional) from an
    /// expression.
    fn parse_class(&mut self, _decl: bool) -> Result<ClassNode, String> {
        self.advance(); // class
        let name = if let Tok::Ident(n) = self.tok() {
            if !is_keyword(n) && n != "extends" {
                let n = n.clone();
                self.advance();
                Some(n)
            } else {
                None
            }
        } else {
            None
        };
        let parent = if self.eat_kw("extends") {
            // The superclass is a left-hand-side expression (`extends Base`,
            // `extends foo.Bar`).
            Some(Box::new(self.parse_call_member()?))
        } else {
            None
        };
        self.expect_punct("{")?;
        let mut members = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            if self.eat_punct(";") {
                continue; // stray semicolons between members
            }
            members.push(self.parse_class_member()?);
        }
        self.expect_punct("}")?;
        Ok(ClassNode { name, parent, members })
    }

    /// Parse one class member: `[static] [get|set|async|*] name(params){…}` or a
    /// `[static] name [= init];` field.
    fn parse_class_member(&mut self) -> Result<ClassMember, String> {
        let is_static = self.is_kw("static")
            && !self.peek_is_member_punct(1)
            && {
                self.advance();
                true
            };
        // Accessor / async / generator prefixes (each contextual: only a prefix
        // when followed by another member name, not itself the member name).
        let mut kind = MemberKind::Method;
        let mut is_async = false;
        let mut is_generator = false;
        if self.is_kw("get") && !self.peek_is_member_punct(1) {
            self.advance();
            kind = MemberKind::Get;
        } else if self.is_kw("set") && !self.peek_is_member_punct(1) {
            self.advance();
            kind = MemberKind::Set;
        } else {
            if self.is_kw("async") && !self.peek_is_member_punct(1) && !self.peek_newline(1) {
                self.advance();
                is_async = true;
            }
            if self.eat_punct("*") {
                is_generator = true;
            }
        }
        // The member key (computed `[expr]`, string, number, or identifier).
        let (key, computed) = self.parse_property_key()?;
        // A field (no parentheses) vs a method.
        if kind == MemberKind::Method && !self.is_punct("(") {
            let field_init = if self.eat_punct("=") {
                Some(self.parse_assign()?)
            } else {
                None
            };
            self.semicolon()?;
            return Ok(ClassMember {
                key,
                computed,
                kind: MemberKind::Field,
                is_static,
                is_generator: false,
                is_async: false,
                params: Vec::new(),
                body: Vec::new(),
                field_init,
            });
        }
        // A method / accessor / constructor.
        let is_ctor = !is_static
            && !computed
            && matches!(&key, Expr::Str(s) if s == "constructor")
            && kind == MemberKind::Method;
        let params = self.parse_params()?;
        self.expect_punct("{")?;
        let body = self.parse_fn_body_block(is_generator, is_async)?;
        Ok(ClassMember {
            key,
            computed,
            kind: if is_ctor { MemberKind::Constructor } else { kind },
            is_static,
            is_generator,
            is_async,
            params,
            body,
            field_init: None,
        })
    }

    /// Whether the token `n` ahead is `(`, `=`, `;`, `}`, or a newline-boundary —
    /// i.e. the current word is itself the member name, not a modifier prefix.
    fn peek_is_member_punct(&self, n: usize) -> bool {
        matches!(
            self.toks.get(self.pos + n).map(|t| &t.tok),
            Some(Tok::Punct(p)) if p == "(" || p == "=" || p == ";" || p == "}"
        )
    }

    /// Parse a property key for a class member / object method: `[expr]` (computed),
    /// a string, a number, or an identifier (returned as an `Expr::Str`).
    fn parse_property_key(&mut self) -> Result<(Expr, bool), String> {
        if self.is_punct("[") {
            self.advance();
            let k = self.parse_assign()?;
            self.expect_punct("]")?;
            Ok((k, true))
        } else {
            match self.tok().clone() {
                Tok::Str(s) => {
                    self.advance();
                    Ok((Expr::Str(s), false))
                }
                Tok::Num(n) => {
                    self.advance();
                    Ok((Expr::Str(crate::host::fmt_number(n)), false))
                }
                Tok::Ident(s) => {
                    self.advance();
                    Ok((Expr::Str(s), false))
                }
                other => Err(format!(
                    "SyntaxError: bad member key {other:?} (line {})",
                    self.line()
                )),
            }
        }
    }

    /// An optional non-newline label after break/continue.
    fn opt_label(&mut self) -> Option<String> {
        if self.newline_before() {
            return None;
        }
        if let Tok::Ident(s) = self.tok() {
            if !is_keyword(s) {
                let s = s.clone();
                self.advance();
                return Some(s);
            }
        }
        None
    }

    /// Parse statements up to (and consuming) the closing `}`.
    fn parse_block_body(&mut self) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            out.push(self.parse_stmt()?);
        }
        self.expect_punct("}")?;
        Ok(out)
    }

    fn parse_decl_kind(&mut self) -> DeclKind {
        let k = match self.tok() {
            Tok::Ident(s) if s == "let" => DeclKind::Let,
            Tok::Ident(s) if s == "const" => DeclKind::Const,
            _ => DeclKind::Var,
        };
        self.advance();
        k
    }

    fn parse_declarators(&mut self) -> Result<Vec<Declarator>, String> {
        let mut decls = Vec::new();
        loop {
            let target = self.parse_binding_target()?;
            let init = if self.eat_punct("=") {
                Some(self.parse_assign()?)
            } else {
                None
            };
            decls.push(Declarator { target, init });
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(decls)
    }

    /// A binding target: identifier or array/object destructuring pattern.
    fn parse_binding_target(&mut self) -> Result<Expr, String> {
        if self.is_punct("[") {
            self.parse_array_literal()
        } else if self.is_punct("{") {
            self.parse_object_literal()
        } else {
            Ok(Expr::Ident(self.ident_name()?))
        }
    }

    fn parse_if(&mut self) -> Result<StmtKind, String> {
        self.advance(); // if
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        let cons = Box::new(self.parse_stmt()?);
        let alt = if self.eat_kw("else") {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(StmtKind::If { test, cons, alt })
    }

    fn parse_while(&mut self) -> Result<StmtKind, String> {
        self.advance();
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        let body = Box::new(self.parse_stmt()?);
        Ok(StmtKind::While { test, body })
    }

    fn parse_do_while(&mut self) -> Result<StmtKind, String> {
        self.advance();
        let body = Box::new(self.parse_stmt()?);
        if !self.eat_kw("while") {
            return Err(format!("SyntaxError: expected 'while' (line {})", self.line()));
        }
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        self.semicolon()?;
        Ok(StmtKind::DoWhile { body, test })
    }

    fn parse_for(&mut self) -> Result<StmtKind, String> {
        self.advance();
        // `for await (… of …)` — the async-iteration form (valid in an async body).
        let is_await = self.eat_kw("await");
        self.expect_punct("(")?;
        // Optional declaration or expression init.
        let decl_kind = match self.tok() {
            Tok::Ident(s) if s == "var" || s == "let" || s == "const" => Some(self.parse_decl_kind()),
            _ => None,
        };
        // Empty init: `for (;;)`.
        if decl_kind.is_none() && self.is_punct(";") {
            return self.parse_c_for(None);
        }
        // Parse the first binding/expression, then decide of/in vs C-style.
        let first_target = if decl_kind.is_some() {
            self.parse_binding_target()?
        } else {
            self.parse_expr_no_in()?
        };
        if self.eat_kw("of") {
            let iter = self.parse_assign()?;
            self.expect_punct(")")?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(StmtKind::ForOf {
                decl_kind,
                target: first_target,
                iter,
                body,
                is_await,
            });
        }
        if self.eat_kw("in") {
            let object = self.parse_assign()?;
            self.expect_punct(")")?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(StmtKind::ForIn {
                decl_kind,
                target: first_target,
                object,
                body,
            });
        }
        // C-style: reconstruct the init statement.
        let init_stmt = if let Some(k) = decl_kind {
            let init = if self.eat_punct("=") {
                Some(self.parse_assign()?)
            } else {
                None
            };
            let mut decls = vec![Declarator { target: first_target, init }];
            while self.eat_punct(",") {
                let target = self.parse_binding_target()?;
                let init = if self.eat_punct("=") {
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                decls.push(Declarator { target, init });
            }
            StmtKind::Decl { kind: k, decls }
        } else {
            StmtKind::Expr(first_target)
        };
        self.parse_c_for(Some(Stmt::from(init_stmt)))
    }

    fn parse_c_for(&mut self, init: Option<Stmt>) -> Result<StmtKind, String> {
        self.expect_punct(";")?;
        let test = if self.is_punct(";") {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_punct(";")?;
        let update = if self.is_punct(")") {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_punct(")")?;
        let body = Box::new(self.parse_stmt()?);
        Ok(StmtKind::For {
            init: init.map(Box::new),
            test,
            update,
            body,
        })
    }

    fn parse_switch(&mut self) -> Result<StmtKind, String> {
        self.advance();
        self.expect_punct("(")?;
        let disc = self.parse_expr()?;
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut cases = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            let test = if self.eat_kw("case") {
                let e = self.parse_expr()?;
                Some(e)
            } else if self.eat_kw("default") {
                None
            } else {
                return Err(format!(
                    "SyntaxError: expected 'case' or 'default' (line {})",
                    self.line()
                ));
            };
            self.expect_punct(":")?;
            let mut body = Vec::new();
            while !self.is_punct("}") && !self.is_kw("case") && !self.is_kw("default") && !self.at_eof() {
                body.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase { test, body });
        }
        self.expect_punct("}")?;
        Ok(StmtKind::Switch { disc, cases })
    }

    fn parse_try(&mut self) -> Result<StmtKind, String> {
        self.advance();
        self.expect_punct("{")?;
        let block = self.parse_block_body()?;
        let handler = if self.eat_kw("catch") {
            let param = if self.eat_punct("(") {
                let p = self.parse_binding_target()?;
                self.expect_punct(")")?;
                Some(p)
            } else {
                None
            };
            self.expect_punct("{")?;
            let body = self.parse_block_body()?;
            Some((param, body))
        } else {
            None
        };
        let finalizer = if self.eat_kw("finally") {
            self.expect_punct("{")?;
            Some(self.parse_block_body()?)
        } else {
            None
        };
        Ok(StmtKind::Try {
            block,
            handler,
            finalizer,
        })
    }

    // ── expressions ──────────────────────────────────────────────────────
    /// Full expression, including the comma sequence operator.
    fn parse_expr(&mut self) -> Result<Expr, String> {
        let first = self.parse_assign()?;
        if self.is_punct(",") {
            let mut items = vec![first];
            while self.eat_punct(",") {
                items.push(self.parse_assign()?);
            }
            Ok(Expr::Sequence(items))
        } else {
            Ok(first)
        }
    }

    /// Like `parse_expr` but stops before `in` (used in `for` init position).
    fn parse_expr_no_in(&mut self) -> Result<Expr, String> {
        // For simplicity the no-in variant only parses an assignment/LHS chain,
        // which is sufficient for `for (x in ...)` / `for (x of ...)` heads.
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        // Arrow function detection.
        if let Some(arrow) = self.try_parse_arrow()? {
            return Ok(arrow);
        }
        let left = self.parse_conditional()?;
        // Assignment operators (right-associative).
        let op = match self.tok() {
            Tok::Punct(p) => p.clone(),
            _ => return Ok(left),
        };
        let compound = match op.as_str() {
            "=" => None,
            "+=" => Some(BinOp::Add),
            "-=" => Some(BinOp::Sub),
            "*=" => Some(BinOp::Mul),
            "/=" => Some(BinOp::Div),
            "%=" => Some(BinOp::Mod),
            "**=" => Some(BinOp::Pow),
            "&=" => Some(BinOp::BitAnd),
            "|=" => Some(BinOp::BitOr),
            "^=" => Some(BinOp::BitXor),
            "<<=" => Some(BinOp::Shl),
            ">>=" => Some(BinOp::Shr),
            ">>>=" => Some(BinOp::UShr),
            "&&=" | "||=" | "??=" => {
                // Logical assignment.
                self.advance();
                let value = self.parse_assign()?;
                let lop = match op.as_str() {
                    "&&=" => LogicalOp::And,
                    "||=" => LogicalOp::Or,
                    _ => LogicalOp::Nullish,
                };
                return Ok(Expr::Assign {
                    target: Box::new(left.clone()),
                    value: Box::new(Expr::Logical(lop, Box::new(left), Box::new(value))),
                });
            }
            _ => return Ok(left),
        };
        self.advance();
        let value = self.parse_assign()?;
        let value = match compound {
            None => value,
            Some(b) => Expr::Binary(b, Box::new(left.clone()), Box::new(value)),
        };
        Ok(Expr::Assign {
            target: Box::new(left),
            value: Box::new(value),
        })
    }

    fn parse_conditional(&mut self) -> Result<Expr, String> {
        let test = self.parse_binary(0)?;
        if self.eat_punct("?") {
            let cons = self.parse_assign()?;
            self.expect_punct(":")?;
            let alt = self.parse_assign()?;
            Ok(Expr::Conditional {
                test: Box::new(test),
                cons: Box::new(cons),
                alt: Box::new(alt),
            })
        } else {
            Ok(test)
        }
    }

    /// Precedence-climbing binary parser. Handles `&& || ??` as logical nodes.
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some((prec, right_assoc, logical, bin)) = self.bin_info() {
            if prec < min_prec {
                break;
            }
            self.advance();
            let next_min = if right_assoc { prec } else { prec + 1 };
            let right = self.parse_binary(next_min)?;
            left = if let Some(lop) = logical {
                Expr::Logical(lop, Box::new(left), Box::new(right))
            } else {
                Expr::Binary(bin.unwrap(), Box::new(left), Box::new(right))
            };
        }
        Ok(left)
    }

    /// `(precedence, right_assoc, logical_op, bin_op)` for the current token.
    fn bin_info(&self) -> Option<(u8, bool, Option<LogicalOp>, Option<BinOp>)> {
        let p = match self.tok() {
            Tok::Punct(p) => p.as_str(),
            Tok::Ident(s) if s == "in" => "in",
            Tok::Ident(s) if s == "instanceof" => "instanceof",
            _ => return None,
        };
        let (prec, ra, log, bin) = match p {
            "??" => (1, false, Some(LogicalOp::Nullish), None),
            "||" => (2, false, Some(LogicalOp::Or), None),
            "&&" => (3, false, Some(LogicalOp::And), None),
            "|" => (4, false, None, Some(BinOp::BitOr)),
            "^" => (5, false, None, Some(BinOp::BitXor)),
            "&" => (6, false, None, Some(BinOp::BitAnd)),
            "==" => (7, false, None, Some(BinOp::EqEq)),
            "!=" => (7, false, None, Some(BinOp::NeEq)),
            "===" => (7, false, None, Some(BinOp::EqEqEq)),
            "!==" => (7, false, None, Some(BinOp::NeEqEq)),
            "<" => (8, false, None, Some(BinOp::Lt)),
            "<=" => (8, false, None, Some(BinOp::Le)),
            ">" => (8, false, None, Some(BinOp::Gt)),
            ">=" => (8, false, None, Some(BinOp::Ge)),
            "in" => (8, false, None, Some(BinOp::In)),
            "instanceof" => (8, false, None, Some(BinOp::InstanceOf)),
            "<<" => (9, false, None, Some(BinOp::Shl)),
            ">>" => (9, false, None, Some(BinOp::Shr)),
            ">>>" => (9, false, None, Some(BinOp::UShr)),
            "+" => (10, false, None, Some(BinOp::Add)),
            "-" => (10, false, None, Some(BinOp::Sub)),
            "*" => (11, false, None, Some(BinOp::Mul)),
            "/" => (11, false, None, Some(BinOp::Div)),
            "%" => (11, false, None, Some(BinOp::Mod)),
            "**" => (12, true, None, Some(BinOp::Pow)),
            _ => return None,
        };
        Some((prec, ra, log, bin))
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        let op = match self.tok() {
            Tok::Punct(p) if p == "!" => Some(UnOp::Not),
            Tok::Punct(p) if p == "~" => Some(UnOp::BitNot),
            Tok::Punct(p) if p == "+" => Some(UnOp::Pos),
            Tok::Punct(p) if p == "-" => Some(UnOp::Neg),
            Tok::Ident(s) if s == "typeof" => Some(UnOp::TypeOf),
            Tok::Ident(s) if s == "void" => Some(UnOp::Void),
            Tok::Ident(s) if s == "delete" => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let e = self.parse_unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        // Prefix ++/--.
        if self.is_punct("++") || self.is_punct("--") {
            let op = if self.is_punct("++") { UpdateOp::Inc } else { UpdateOp::Dec };
            self.advance();
            let e = self.parse_unary()?;
            return Ok(Expr::Update {
                op,
                prefix: true,
                target: Box::new(e),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_call_member()?;
        // Postfix ++/-- (no line break before).
        if (self.is_punct("++") || self.is_punct("--")) && !self.newline_before() {
            let op = if self.is_punct("++") { UpdateOp::Inc } else { UpdateOp::Dec };
            self.advance();
            e = Expr::Update {
                op,
                prefix: false,
                target: Box::new(e),
            };
        }
        Ok(e)
    }

    fn parse_call_member(&mut self) -> Result<Expr, String> {
        let mut e = if self.eat_kw("new") {
            // `new.target` meta-property.
            if self.is_punct(".") {
                self.advance();
                let prop = self.ident_name()?;
                if prop != "target" {
                    return Err(format!("SyntaxError: expected 'target' (line {})", self.line()));
                }
                Expr::NewTarget
            } else {
                let callee = self.parse_call_member_no_call()?;
                let args = if self.is_punct("(") {
                    self.parse_args()?
                } else {
                    Vec::new()
                };
                Expr::New {
                    callee: Box::new(callee),
                    args,
                }
            }
        } else {
            self.parse_primary()?
        };
        loop {
            if self.eat_punct(".") {
                let property = self.ident_name()?;
                e = Expr::Member {
                    object: Box::new(e),
                    property,
                    optional: false,
                };
            } else if self.eat_punct("?.") {
                if self.is_punct("(") {
                    let args = self.parse_args()?;
                    e = Expr::Call {
                        func: Box::new(e),
                        args,
                        optional: true,
                    };
                } else if self.is_punct("[") {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect_punct("]")?;
                    e = Expr::Index {
                        object: Box::new(e),
                        index: Box::new(index),
                        optional: true,
                    };
                } else {
                    let property = self.ident_name()?;
                    e = Expr::Member {
                        object: Box::new(e),
                        property,
                        optional: true,
                    };
                }
            } else if self.is_punct("[") {
                self.advance();
                let index = self.parse_expr()?;
                self.expect_punct("]")?;
                e = Expr::Index {
                    object: Box::new(e),
                    index: Box::new(index),
                    optional: false,
                };
            } else if self.is_punct("(") {
                let args = self.parse_args()?;
                e = Expr::Call {
                    func: Box::new(e),
                    args,
                    optional: false,
                };
            } else if matches!(self.tok(), Tok::Template { .. }) {
                // A template literal immediately after a callee is a *tagged*
                // template: `` tag`...` `` → `tag(strings, ...values)`.
                e = self.parse_tagged_template(e)?;
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// Parse `` tag`a${x}b` `` into a `TaggedTemplate` node (the tag expression is
    /// already parsed as `tag`, and the current token is the template).
    fn parse_tagged_template(&mut self, tag: Expr) -> Result<Expr, String> {
        let (quasis, raws, exprs_src) = match self.tok().clone() {
            Tok::Template { quasis, raws, exprs } => (quasis, raws, exprs),
            _ => unreachable!(),
        };
        self.advance();
        let mut exprs = Vec::new();
        for src in &exprs_src {
            exprs.push(parse_expr_source(src)?);
        }
        Ok(Expr::TaggedTemplate {
            tag: Box::new(tag),
            quasis,
            raws,
            exprs,
        })
    }

    /// Member chain without a trailing call — the `new X.Y` callee grammar.
    fn parse_call_member_no_call(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat_punct(".") {
                let property = self.ident_name()?;
                e = Expr::Member {
                    object: Box::new(e),
                    property,
                    optional: false,
                };
            } else if self.is_punct("[") {
                self.advance();
                let index = self.parse_expr()?;
                self.expect_punct("]")?;
                e = Expr::Index {
                    object: Box::new(e),
                    index: Box::new(index),
                    optional: false,
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        while !self.is_punct(")") {
            if self.eat_punct("...") {
                let e = self.parse_assign()?;
                args.push(Expr::Spread(Box::new(e)));
            } else {
                args.push(self.parse_assign()?);
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.tok().clone() {
            Tok::Num(n) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Tok::BigInt(s) => {
                self.advance();
                Ok(Expr::BigInt(s))
            }
            Tok::Regex(pat, flags) => {
                self.advance();
                Ok(Expr::Regex(pat, flags))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Tok::Template { quasis, raws: _, exprs } => {
                self.advance();
                let mut parsed = Vec::new();
                for src in &exprs {
                    parsed.push(parse_expr_source(src)?);
                }
                Ok(Expr::Template {
                    quasis,
                    exprs: parsed,
                })
            }
            Tok::Punct(p) if p == "(" => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Punct(p) if p == "[" => self.parse_array_literal(),
            Tok::Punct(p) if p == "{" => self.parse_object_literal(),
            Tok::Ident(s) => {
                match s.as_str() {
                    "true" => {
                        self.advance();
                        Ok(Expr::True)
                    }
                    "false" => {
                        self.advance();
                        Ok(Expr::False)
                    }
                    "null" => {
                        self.advance();
                        Ok(Expr::Null)
                    }
                    "this" => {
                        self.advance();
                        Ok(Expr::This)
                    }
                    "super" => {
                        self.advance();
                        Ok(Expr::Super)
                    }
                    "class" => Ok(Expr::Class(Box::new(self.parse_class(false)?))),
                    "function" => self.parse_function_expr(false),
                    "async" if self.peek_kw(1, "function") && !self.peek_newline(1) => {
                        self.advance(); // async
                        self.parse_function_expr(true)
                    }
                    "yield" if self.in_generator => {
                        self.advance();
                        let delegate = self.eat_punct("*");
                        // `yield` with no argument (before `)`, `]`, `}`, `,`, `;`,
                        // newline, or EOF).
                        let arg = if delegate
                            || !(self.is_punct(")")
                                || self.is_punct("]")
                                || self.is_punct("}")
                                || self.is_punct(",")
                                || self.is_punct(";")
                                || self.is_punct(":")
                                || self.newline_before()
                                || self.at_eof())
                        {
                            Some(Box::new(self.parse_assign()?))
                        } else {
                            None
                        };
                        Ok(Expr::Yield { arg, delegate })
                    }
                    "await" if self.in_async => {
                        self.advance();
                        let e = self.parse_unary()?;
                        Ok(Expr::Await(Box::new(e)))
                    }
                    _ if is_keyword(&s) => Err(format!(
                        "SyntaxError: unexpected keyword '{s}' (line {})",
                        self.line()
                    )),
                    _ => {
                        self.advance();
                        Ok(Expr::Ident(s))
                    }
                }
            }
            other => Err(format!(
                "SyntaxError: unexpected token {other:?} (line {})",
                self.line()
            )),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, String> {
        self.expect_punct("[")?;
        let mut items = Vec::new();
        while !self.is_punct("]") {
            if self.is_punct(",") {
                // Elision (hole) — represent as undefined.
                items.push(Expr::Undefined);
                self.advance();
                continue;
            }
            if self.eat_punct("...") {
                let e = self.parse_assign()?;
                items.push(Expr::Spread(Box::new(e)));
            } else {
                items.push(self.parse_assign()?);
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("]")?;
        Ok(Expr::Array(items))
    }

    fn parse_object_literal(&mut self) -> Result<Expr, String> {
        self.expect_punct("{")?;
        let mut props = Vec::new();
        while !self.is_punct("}") {
            if self.eat_punct("...") {
                let e = self.parse_assign()?;
                props.push(Prop::Spread(e));
                if !self.eat_punct(",") {
                    break;
                }
                continue;
            }
            // `get key() {}` / `set key(v) {}` accessor (contextual: `get`/`set`
            // is a modifier only when followed by another key, not `:`/`(`/`,`).
            if (self.is_kw("get") || self.is_kw("set")) && !self.peek_is_member_punct(1)
                && !matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == ":" || p == ",")
            {
                let is_getter = self.is_kw("get");
                self.advance();
                let (key, computed) = self.parse_property_key()?;
                let params = self.parse_params()?;
                self.expect_punct("{")?;
                let body = self.parse_block_body()?;
                let func = Expr::Function {
                    params,
                    body: FnBody::Block(body),
                    is_arrow: false,
                    name: None,
                    is_generator: false,
                    is_async: false,
                };
                props.push(Prop::Accessor {
                    key,
                    computed,
                    is_getter,
                    func,
                });
                if !self.eat_punct(",") {
                    break;
                }
                continue;
            }
            // Concise-method modifiers: `async` and/or `*` before the key.
            let mut m_async = false;
            let mut m_gen = false;
            if self.is_kw("async") && !self.peek_is_member_punct(1) && !self.peek_newline(1)
                && !matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == ":" || p == ",")
            {
                self.advance();
                m_async = true;
            }
            if self.is_punct("*") {
                self.advance();
                m_gen = true;
            }
            let (key, computed) = self.parse_property_key()?;
            // Method shorthand `key(params) { }` (incl. `*gen(){}`, `async m(){}`).
            if self.is_punct("(") {
                let params = self.parse_params()?;
                self.expect_punct("{")?;
                let body = self.parse_fn_body_block(m_gen, m_async)?;
                let f = Expr::Function {
                    params,
                    body: FnBody::Block(body),
                    is_arrow: false,
                    name: None,
                    is_generator: m_gen,
                    is_async: m_async,
                };
                props.push(Prop::KeyValue {
                    key,
                    value: f,
                    computed,
                });
            } else if self.eat_punct(":") {
                let value = self.parse_assign()?;
                props.push(Prop::KeyValue {
                    key,
                    value,
                    computed,
                });
            } else {
                // Shorthand `{ x }` -> key "x", value ident x. Or with default
                // in a destructuring pattern: `{ x = 1 }`.
                let name = match &key {
                    Expr::Str(s) => s.clone(),
                    _ => return Err(format!("SyntaxError: bad shorthand (line {})", self.line())),
                };
                let value = if self.eat_punct("=") {
                    // Pattern default; represent as Assign so destructuring reads it.
                    let d = self.parse_assign()?;
                    Expr::Assign {
                        target: Box::new(Expr::Ident(name.clone())),
                        value: Box::new(d),
                    }
                } else {
                    Expr::Ident(name)
                };
                props.push(Prop::KeyValue {
                    key,
                    value,
                    computed,
                });
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}")?;
        Ok(Expr::Object(props))
    }

    // ── functions / arrows ───────────────────────────────────────────────
    fn parse_params(&mut self) -> Result<Vec<Param>, String> {
        self.expect_punct("(")?;
        let mut params = Vec::new();
        while !self.is_punct(")") {
            let rest = self.eat_punct("...");
            let pattern = self.parse_binding_target()?;
            let default = if !rest && self.eat_punct("=") {
                Some(self.parse_assign()?)
            } else {
                None
            };
            params.push(Param {
                pattern,
                default,
                rest,
            });
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(params)
    }

    /// Try to parse an arrow function starting at the current position. Returns
    /// `None` (without consuming) if the head is not an arrow.
    fn try_parse_arrow(&mut self) -> Result<Option<Expr>, String> {
        // `async` prefix on an arrow (`async x => …` / `async (…) => …`), only
        // when `async` is not itself the parameter and no newline intervenes.
        let mut is_async = false;
        let mut base = self.pos;
        if self.is_kw("async") && !self.peek_newline(1) {
            let next = self.toks.get(self.pos + 1).map(|t| &t.tok);
            let looks_async_arrow = matches!(next, Some(Tok::Punct(p)) if p == "(")
                || matches!(next, Some(Tok::Ident(n)) if !is_keyword(n) && self.peek_is_arrow_after(2));
            if looks_async_arrow {
                is_async = true;
                base += 1;
            }
        }
        // `ident => ...`
        if let Some(Tok::Ident(name)) = self.toks.get(base).map(|t| &t.tok) {
            if !is_keyword(name) && matches!(self.toks.get(base + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "=>") {
                let name = name.clone();
                if is_async {
                    self.advance(); // async
                }
                self.advance(); // ident
                self.advance(); // =>
                let body = self.parse_arrow_body(is_async)?;
                return Ok(Some(Expr::Function {
                    params: vec![Param {
                        pattern: Expr::Ident(name),
                        default: None,
                        rest: false,
                    }],
                    body,
                    is_arrow: true,
                    name: None,
                    is_generator: false,
                    is_async,
                }));
            }
        }
        // `( ... ) => ...`
        if matches!(self.toks.get(base).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "(") {
            if let Some(close) = self.matching_paren(base) {
                let after = close + 1;
                if matches!(self.toks.get(after).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "=>") {
                    if is_async {
                        self.advance(); // async
                    }
                    let params = self.parse_params()?;
                    self.expect_punct("=>")?;
                    let body = self.parse_arrow_body(is_async)?;
                    return Ok(Some(Expr::Function {
                        params,
                        body,
                        is_arrow: true,
                        name: None,
                        is_generator: false,
                        is_async,
                    }));
                }
            }
        }
        Ok(None)
    }

    fn parse_arrow_body(&mut self, is_async: bool) -> Result<FnBody, String> {
        let (pg, pa) = (self.in_generator, self.in_async);
        self.in_generator = false;
        self.in_async = is_async;
        let r = if self.is_punct("{") {
            self.advance();
            self.parse_block_body().map(FnBody::Block)
        } else {
            self.parse_assign().map(|e| FnBody::Expr(Box::new(e)))
        };
        self.in_generator = pg;
        self.in_async = pa;
        r
    }

    /// Whether the token `n` positions ahead is `=>`.
    fn peek_is_arrow_after(&self, n: usize) -> bool {
        matches!(self.toks.get(self.pos + n).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "=>")
    }

    /// Index of the `)` matching the `(` at `open`, skipping nested brackets.
    fn matching_paren(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        let mut i = open;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Punct(p) if p == "(" || p == "[" || p == "{" => depth += 1,
                Tok::Punct(p) if p == ")" || p == "]" || p == "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
            i += 1;
        }
        None
    }
}

/// Parse a template-literal `${...}` field's raw source into an expression.
fn parse_expr_source(src: &str) -> Result<Expr, String> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0, in_generator: false, in_async: false };
    let e = p.parse_expr()?;
    Ok(e)
}
