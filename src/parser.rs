//! JavaScript parser: token stream → AST.
//!
//! Recursive descent with precedence climbing for binary operators. Automatic
//! Semicolon Insertion is applied at statement boundaries using the
//! `newline_before` flag the lexer records on every token. Arrow functions are
//! detected at assignment level by looking ahead for `=>` after a parameter
//! list. Template-literal `${...}` fields are re-parsed here from the raw source
//! the lexer captured.

use crate::ast::*;
use crate::lexer::{lex, Legacy, Tok, Token};

const KEYWORDS: &[&str] = &[
    "var",
    "let",
    "const",
    "function",
    "return",
    "if",
    "else",
    "while",
    "do",
    "for",
    "of",
    "in",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "true",
    "false",
    "null",
    "this",
    "new",
    "typeof",
    "void",
    "delete",
    "instanceof",
    "throw",
    "try",
    "catch",
    "finally",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

/// The private names one class body declares and the ones its code refers to.
#[derive(Default)]
struct PrivateScope {
    declared: Vec<String>,
    used: Vec<String>,
}

/// The names a binding pattern introduces.
fn pattern_idents(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ident(n) => out.push(n.clone()),
        Expr::Array(items) => items.iter().for_each(|i| pattern_idents(i, out)),
        Expr::Spread(inner) => pattern_idents(inner, out),
        Expr::Assign { target, .. } => pattern_idents(target, out),
        Expr::Object(props) => {
            for pr in props {
                match pr {
                    Prop::KeyValue { value, .. } => pattern_idents(value, out),
                    Prop::Spread(inner) => pattern_idents(inner, out),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// 15.4.1: a getter takes no parameters and a setter exactly one, which is not
/// a rest parameter.
fn check_accessor_arity(kind: MemberKind, params: &[Param]) -> Result<(), String> {
    match kind {
        MemberKind::Get if !params.is_empty() => {
            Err("SyntaxError: Getter must not have any formal parameters.".to_string())
        }
        MemberKind::Set if params.len() != 1 => {
            Err("SyntaxError: Setter must have exactly one formal parameter.".to_string())
        }
        MemberKind::Set if params[0].rest => {
            Err("SyntaxError: Setter function argument must not be a rest parameter".to_string())
        }
        _ => Ok(()),
    }
}

fn private_not_declared(name: &str) -> String {
    format!("SyntaxError: Private field '{name}' must be declared in an enclosing class")
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// True while parsing a generator body — enables `yield` as an operator.
    in_generator: bool,
    /// True while parsing an async body — enables `await` as an operator.
    in_async: bool,
    /// True while parsing a `for` init in LHS position — suppresses `in` as a
    /// relational operator so `for (x in obj)` (no declaration keyword) parses
    /// the `in` as the loop separator, not a binary expression. Cleared inside
    /// any parenthesised/bracketed sub-expression, where `in` is legal again.
    no_in: bool,
    /// One frame per enclosing class body, each holding the private names that
    /// body DECLARES and the ones its code USES. A private name is only in
    /// scope inside a class that declares it, so an undeclared use is an EARLY
    /// error (`SyntaxError`) rather than a runtime `TypeError` if the read ever
    /// happens — a typo in a dead branch used to ship silently.
    class_scopes: Vec<PrivateScope>,
    /// Whether token offsets point into the script text, so a function's
    /// [`Span`] can be recorded. Off for a re-parsed template field.
    spans: bool,
    /// The labels of the statements enclosing the current one, innermost last,
    /// reset at each function boundary: a label may not be declared twice in
    /// one nest (14.13.1).
    labels: Vec<String>,
    /// Whether the code being parsed is strict (a `'use strict'` directive
    /// prologue, or a class body). Decides the early errors that only strict
    /// code has: legacy octal forms, `let`/`yield`/… as names, a function
    /// declaration as an `if`/loop body.
    strict: bool,
    /// Whether `super` may appear here: a method, an accessor, a class field
    /// initialiser or a static block, and an arrow inside one. An `eval`'s
    /// source starts out true, since it may be running inside a method.
    super_ok: bool,
    /// Whether a `return` is legal here: inside a function body, or at the top
    /// of a file (which Node wraps in one). An `eval`'s top level is not.
    in_function: bool,
    /// Token positions of `{ a = 1 }` shorthand initialisers seen in an
    /// expression. They are legal only if the object turns out to be a pattern
    /// (an assignment target, a `for` head); anything left when the statement
    /// ends is the `Invalid shorthand property initializer` early error.
    cover_init: Vec<usize>,
    /// Set while parsing a binding pattern, where `{ a = 1 }` is simply a
    /// default and never a cover grammar.
    in_binding: bool,
}

/// Parse a complete JS program into a statement list. Inline `rust { ... }` FFI
/// blocks are desugared to `__rust_compile(...)` calls before lexing.
pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    parse_source(src, false, false, false)
}

/// Parse the source text of an `eval` / `vm` script. Identical to [`parse`]
/// except that `super` is accepted at the top level when `allow_super` says the
/// code runs inside a method (a direct `eval` there may use it).
pub fn parse_eval(src: &str, allow_super: bool, caller_strict: bool) -> Result<Vec<Stmt>, String> {
    parse_source(src, allow_super, true, caller_strict)
}

fn parse_source(
    src: &str,
    allow_super: bool,
    in_eval: bool,
    caller_strict: bool,
) -> Result<Vec<Stmt>, String> {
    let desugared = crate::rust_ffi::desugar(src);
    // A `rust { … }` block is rewritten before lexing, so offsets would point
    // into text the caller never sees; record spans only when nothing moved.
    let spans = desugared == src;
    let toks = lex(&desugared)?;
    let mut p = Parser {
        toks,
        pos: 0,
        in_generator: false,
        in_async: false,
        no_in: false,
        class_scopes: Vec::new(),
        labels: Vec::new(),
        strict: false,
        super_ok: allow_super,
        in_function: !in_eval,
        cover_init: Vec::new(),
        in_binding: false,
        spans,
    };
    p.strict = caller_strict || p.leading_use_strict(0);
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
    /// Parse one `${…}` field of a template token, keeping its spans in this
    /// script's coordinates. The field is part of the enclosing code, so it
    /// parses in the same context: `await` and `yield` stay operators inside
    /// an async or generator body, and a private name it uses is checked
    /// against the enclosing class body like any other use. A field used to be
    /// parsed as if at the top level, so `` `${await x}` `` was a
    /// `ReferenceError` and `` `${this.#c}` `` a `SyntaxError`.
    fn parse_field(&mut self, src: &str, at: u32) -> Result<Expr, String> {
        // `${}`: the field is empty, so the first thing wrong is its `}`.
        if src.trim().is_empty() {
            return Err("SyntaxError: Unexpected token '}'".to_string());
        }
        let mut p = field_parser(src, self.spans.then_some(at))?;
        p.in_generator = self.in_generator;
        p.in_async = self.in_async;
        p.strict = self.strict;
        p.super_ok = self.super_ok;
        p.in_function = self.in_function;
        if !self.class_scopes.is_empty() {
            p.class_scopes.push(PrivateScope::default());
        }
        let e = p.parse_expr()?;
        if let (Some(scope), Some(inner)) = (self.class_scopes.last_mut(), p.class_scopes.pop()) {
            scope.used.extend(inner.used);
        }
        Ok(e)
    }

    /// Byte offset where token `i` begins.
    fn start_at(&self, i: usize) -> u32 {
        self.toks[i].start
    }
    /// The source range from `start` to the end of the last consumed token.
    fn span_from(&self, start: u32) -> Span {
        if !self.spans || self.pos == 0 {
            return (0, 0);
        }
        (start, self.toks[self.pos - 1].end)
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
    /// V8's message for a token the grammar has no place for. Which wording
    /// depends on the token's class — a reserved word and a punctuator are
    /// `Unexpected token 'x'`, an identifier names itself, and a literal says
    /// only what kind it is — so every "this token cannot go here" error site
    /// shares one function rather than describing the token its own way.
    fn unexpected(&self) -> String {
        /// The ReservedWords (13.1.1) V8 reports as a bare token.
        const RESERVED: &[&str] = &[
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "new",
            "null",
            "return",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
        ];
        // V8 explains a stray `await` operand by the missing `async`, not by
        // the operand: `await 1` outside an async function reaches the `1`.
        if self.pos > 0
            && !self.in_async
            && matches!(&self.toks[self.pos - 1].tok, Tok::Ident(s) if s == "await")
            && !matches!(self.tok(), Tok::Punct(p) if p == ")" || p == ";" || p == ",")
        {
            return "SyntaxError: await is only valid in async functions and the top level bodies of modules".to_string();
        }
        let what = match self.tok() {
            Tok::Eof => "Unexpected end of input".to_string(),
            Tok::Num(_) | Tok::BigInt(_) => "Unexpected number".to_string(),
            Tok::Str(_) => "Unexpected string".to_string(),
            Tok::Template { .. } => "Unexpected template string".to_string(),
            Tok::Regex(..) => "Unexpected regular expression".to_string(),
            Tok::Ident(s) if s == "enum" => "Unexpected reserved word".to_string(),
            Tok::Ident(s) if RESERVED.contains(&s.as_str()) => {
                format!("Unexpected token '{s}'")
            }
            Tok::Ident(s) => format!("Unexpected identifier '{s}'"),
            Tok::Punct(p) => format!("Unexpected token '{p}'"),
        };
        format!("SyntaxError: {what}")
    }

    /// The parameter-list early errors that depend on how the function is
    /// written (15.2.1, 15.1.1): a repeated name is allowed only in sloppy code
    /// with a plain list of simple names, and a `'use strict'` body needs a
    /// simple list. `unique` is set for arrows, methods and accessors, which
    /// never allow a repeat; `body_strict` for a body that begins with the
    /// directive (strictness inherited from outside is `self.strict`).
    fn check_params(
        &self,
        params: &[Param],
        body_strict: bool,
        unique: bool,
    ) -> Result<(), String> {
        let simple = params
            .iter()
            .all(|p| !p.rest && p.default.is_none() && matches!(p.pattern, Expr::Ident(_)));
        if body_strict && !simple {
            return Err(
                "SyntaxError: Illegal 'use strict' directive in function with \
                        non-simple parameter list"
                    .to_string(),
            );
        }
        let strict = self.strict || body_strict;
        let mut names: Vec<String> = Vec::new();
        for p in params {
            pattern_idents(&p.pattern, &mut names);
        }
        if strict && names.iter().any(|n| n == "eval" || n == "arguments") {
            return Err("SyntaxError: Unexpected eval or arguments in strict mode".to_string());
        }
        let repeats = names
            .iter()
            .enumerate()
            .any(|(i, n)| names[..i].contains(n));
        if repeats && (strict || unique || !simple) {
            return Err(
                "SyntaxError: Duplicate parameter name not allowed in this context".to_string(),
            );
        }
        Ok(())
    }

    /// Whether the directive prologue starting at token `from` contains
    /// `'use strict'` (14.1.1): the leading run of string-literal expression
    /// statements, each ended by `;`, a newline or the end of the block.
    fn leading_use_strict(&self, from: usize) -> bool {
        let mut i = from;
        while let Some(Token {
            tok: Tok::Str(s), ..
        }) = self.toks.get(i)
        {
            let ends = match self.toks.get(i + 1) {
                Some(t) => {
                    matches!(&t.tok, Tok::Punct(p) if p == ";" || p == "}")
                        || t.newline_before
                        || matches!(t.tok, Tok::Eof)
                }
                None => true,
            };
            if !ends {
                return false;
            }
            if s == "use strict" {
                return true;
            }
            i += 1;
            if matches!(self.toks.get(i), Some(Token { tok: Tok::Punct(p), .. }) if p == ";") {
                i += 1;
            }
        }
        false
    }

    /// Run `f` with `super` allowed or not.
    fn with_super<T>(&mut self, ok: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.super_ok, ok);
        let out = f(self);
        self.super_ok = saved;
        out
    }

    /// The strict-mode-only restriction the current token trips, if the code
    /// is strict.
    fn check_legacy(&self) -> Result<(), String> {
        if !self.strict {
            return Ok(());
        }
        let msg = match self.cur().legacy {
            Legacy::None => return Ok(()),
            Legacy::OctalLiteral => "Octal literals are not allowed in strict mode.",
            Legacy::LeadingZeroDecimal => {
                "Decimals with leading zeros are not allowed in strict mode."
            }
            Legacy::OctalEscape => "Octal escape sequences are not allowed in strict mode.",
            Legacy::NonOctalEscape => "\\8 and \\9 are not allowed in strict mode.",
        };
        Err(format!("SyntaxError: {msg}"))
    }

    fn expect_punct(&mut self, s: &str) -> Result<(), String> {
        if self.eat_punct(s) {
            Ok(())
        } else {
            Err(self.unexpected())
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
            _ => Err(self.unexpected()),
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
        Err(self.unexpected())
    }

    // ── statements ───────────────────────────────────────────────────────
    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        let stmt = self.parse_stmt_inner()?;
        // Whatever `{ a = 1 }` is still unclaimed by a pattern is an error
        // (13.2.5.1): report it where V8 does, at the statement's end.
        if !self.cover_init.is_empty() {
            return Err("SyntaxError: Invalid shorthand property initializer".to_string());
        }
        Ok(stmt)
    }

    fn parse_stmt_inner(&mut self) -> Result<Stmt, String> {
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
            // A script has no `export`, and `import` is only the call form
            // `import(…)` and the meta property `import.meta`.
            Tok::Ident(kw) if kw == "export" => return Err(self.unexpected()),
            Tok::Ident(kw)
                if kw == "import"
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Punct(p)) if p == "(" || p == "."
                    ) =>
            {
                return Err("SyntaxError: Cannot use import statement outside a module".to_string());
            }
            Tok::Ident(kw) if kw == "var" || kw == "let" || kw == "const" => {
                let k = self.parse_decl_kind();
                let decls = self.parse_declarators()?;
                self.check_declarators(&k, &decls)?;
                self.semicolon()?;
                StmtKind::Decl { kind: k, decls }
            }
            Tok::Ident(kw) if kw == "function" => self.parse_func_decl(false)?,
            // `async function …` (declaration). `async` stays a plain identifier
            // anywhere else (contextual keyword).
            Tok::Ident(kw)
                if kw == "async" && self.peek_kw(1, "function") && !self.peek_newline(1) =>
            {
                self.advance(); // async
                self.parse_func_decl(true)?
            }
            Tok::Ident(kw) if kw == "class" => {
                // A class DECLARATION needs a binding name (15.7.1).
                if matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::Punct(p)) if p == "{"
                ) || self.peek_kw(1, "extends")
                {
                    self.advance();
                    return Err(self.unexpected());
                }
                let node = self.parse_class(true)?;
                StmtKind::ClassDecl(node)
            }
            Tok::Ident(kw) if kw == "if" => self.parse_if()?,
            Tok::Ident(kw) if kw == "while" => self.parse_while()?,
            Tok::Ident(kw) if kw == "with" => self.parse_with()?,
            Tok::Ident(kw) if kw == "do" => self.parse_do_while()?,
            Tok::Ident(kw) if kw == "for" => self.parse_for()?,
            Tok::Ident(kw) if kw == "switch" => self.parse_switch()?,
            Tok::Ident(kw) if kw == "return" => {
                if !self.in_function {
                    return Err("SyntaxError: Illegal return statement".to_string());
                }
                self.advance();
                let arg = if self.is_punct(";")
                    || self.is_punct("}")
                    || self.newline_before()
                    || self.at_eof()
                {
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
                // 14.14: no LineTerminator between `throw` and its operand.
                if self.newline_before() {
                    return Err("SyntaxError: Illegal newline after throw".to_string());
                }
                let e = self.parse_expr()?;
                self.semicolon()?;
                StmtKind::Throw(e)
            }
            Tok::Ident(kw) if kw == "try" => self.parse_try()?,
            // `label: stmt` — a bare identifier immediately followed by `:` at
            // statement position is a label (never an expression; the reserved
            // control keywords are all matched above, and switch `case`/`default`
            // labels are parsed inside `parse_switch`).
            Tok::Ident(name) if matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == ":") =>
            {
                if self.labels.contains(&name) {
                    return Err(format!(
                        "SyntaxError: Label '{name}' has already been declared"
                    ));
                }
                self.advance(); // the label identifier
                self.advance(); // the ':'
                self.labels.push(name.clone());
                // 14.13.1 / B.3.2: a labelled FunctionDeclaration is sloppy-only.
                let body = if self.strict
                    && (self.is_kw("function")
                        || self.is_kw("async") && self.peek_kw(1, "function"))
                {
                    Err("SyntaxError: In strict mode code, functions can only be declared at top level or inside a block.".to_string())
                } else {
                    self.parse_stmt()
                };
                self.labels.pop();
                StmtKind::Labeled {
                    label: name,
                    body: Box::new(body?),
                }
            }
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
        self.toks
            .get(self.pos + n)
            .map(|t| t.newline_before)
            .unwrap_or(false)
    }

    /// Parse a `function` declaration (the `function`/`async function` keyword is
    /// current). `is_async` is true when a preceding `async` was consumed.
    fn parse_func_decl(&mut self, is_async: bool) -> Result<StmtKind, String> {
        // `async` was consumed by the caller and is part of the source text.
        let start = self.start_at(if is_async { self.pos - 1 } else { self.pos });
        self.advance(); // function
        let is_generator = self.eat_punct("*");
        if self.is_punct("(") {
            return Err("SyntaxError: Function statements require a function name".to_string());
        }
        let name = self.ident_name()?;
        let params = self.parse_params()?;
        self.check_params(&params, self.leading_use_strict(self.pos + 1), false)?;
        self.expect_punct("{")?;
        let body = self.with_super(false, |p| p.parse_fn_body_block(is_generator, is_async))?;
        Ok(StmtKind::FuncDecl {
            name,
            params,
            body,
            is_generator,
            is_async,
            span: self.span_from(start),
        })
    }

    /// Parse a brace-delimited function body under the given generator/async
    /// context (so `yield`/`await` inside are operators, not identifiers).
    fn parse_fn_body_block(
        &mut self,
        is_generator: bool,
        is_async: bool,
    ) -> Result<Vec<Stmt>, String> {
        self.parse_body_block(is_generator, is_async, true)
    }

    /// [`Self::parse_fn_body_block`] with `return` admitted or not: a class
    /// static block is a body in every other respect but may not `return`.
    fn parse_body_block(
        &mut self,
        is_generator: bool,
        is_async: bool,
        returns: bool,
    ) -> Result<Vec<Stmt>, String> {
        let (pg, pa) = (self.in_generator, self.in_async);
        self.in_generator = is_generator;
        self.in_async = is_async;
        let outer_labels = std::mem::take(&mut self.labels);
        let outer_strict = self.strict;
        let outer_in_function = std::mem::replace(&mut self.in_function, returns);
        self.strict = outer_strict || self.leading_use_strict(self.pos);
        let body = self.parse_block_body();
        self.in_function = outer_in_function;
        self.strict = outer_strict;
        self.labels = outer_labels;
        self.in_generator = pg;
        self.in_async = pa;
        body
    }

    /// Parse a function *expression* (`function`/`async function`, keyword
    /// current). Supports `function*` generators.
    fn parse_function_expr(&mut self, is_async: bool) -> Result<Expr, String> {
        let start = self.start_at(if is_async { self.pos - 1 } else { self.pos });
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
        self.check_params(&params, self.leading_use_strict(self.pos + 1), false)?;
        self.expect_punct("{")?;
        let body = self.with_super(false, |p| p.parse_fn_body_block(is_generator, is_async))?;
        Ok(Expr::Function {
            params,
            body: FnBody::Block(body),
            is_arrow: false,
            name,
            is_generator,
            is_async,
            is_method: false,
            span: self.span_from(start),
        })
    }

    /// Parse a `class` (the `class` keyword is current). `_decl` distinguishes a
    /// declaration (name required in strict mode, but we accept optional) from an
    /// expression.
    /// 15.7.1: a private name must be declared by an enclosing class body.
    /// Outside one, `this.#x` is a SyntaxError at parse time — it used to parse
    /// and then throw a `TypeError` only if the read ever ran, so a typo in a
    /// dead branch shipped silently.
    fn check_private_in_scope(&mut self, property: &str) -> Result<(), String> {
        if !property.starts_with('#') {
            return Ok(());
        }
        match self.class_scopes.last_mut() {
            // Recorded, not resolved: the declaration may still be ahead of the
            // use in the same body, so the check runs when the body closes.
            Some(scope) => scope.used.push(property.to_string()),
            None => return Err(private_not_declared(property)),
        }
        Ok(())
    }

    fn parse_class(&mut self, _decl: bool) -> Result<ClassNode, String> {
        self.class_scopes.push(PrivateScope::default());
        let out = self.parse_class_inner();
        let scope = self.class_scopes.pop().unwrap_or_default();
        let out = out?;
        for name in &scope.used {
            if scope.declared.contains(name) {
                continue;
            }
            // A nested class may reference a name an OUTER class declares, so an
            // unresolved use is handed outwards rather than rejected here.
            match self.class_scopes.last_mut() {
                Some(outer) => outer.used.push(name.clone()),
                None => return Err(private_not_declared(name)),
            }
        }
        Ok(out)
    }

    fn parse_class_inner(&mut self) -> Result<ClassNode, String> {
        let start = self.start_at(self.pos);
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
        // 11.2.2: every part of a class is strict code.
        let outer_strict = std::mem::replace(&mut self.strict, true);
        let mut members = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            if self.eat_punct(";") {
                continue; // stray semicolons between members
            }
            let member = self.with_super(true, |p| p.parse_class_member());
            let member = match member {
                Ok(m) => m,
                Err(e) => {
                    self.strict = outer_strict;
                    return Err(e);
                }
            };
            if member.kind == MemberKind::Constructor
                && members
                    .iter()
                    .any(|m: &ClassMember| m.kind == MemberKind::Constructor)
            {
                return Err("SyntaxError: A class may only have one constructor".to_string());
            }
            if let Expr::Str(k) = &member.key {
                if k.starts_with('#') {
                    // A name is declared once — except a getter and a setter of
                    // the same staticness, which together are one accessor.
                    let pair = |a: &MemberKind, b: &MemberKind| {
                        matches!(
                            (a, b),
                            (MemberKind::Get, MemberKind::Set) | (MemberKind::Set, MemberKind::Get)
                        )
                    };
                    let clash = members.iter().any(|m: &ClassMember| {
                        matches!(&m.key, Expr::Str(n) if n == k)
                            && !(pair(&m.kind, &member.kind) && m.is_static == member.is_static)
                    });
                    if clash {
                        return Err(format!(
                            "SyntaxError: Identifier '{k}' has already been declared"
                        ));
                    }
                    if let Some(scope) = self.class_scopes.last_mut() {
                        scope.declared.push(k.clone());
                    }
                }
            }
            members.push(member);
        }
        self.strict = outer_strict;
        self.expect_punct("}")?;
        Ok(ClassNode {
            name,
            parent,
            members,
            span: self.span_from(start),
        })
    }

    /// Parse one class member: `[static] [get|set|async|*] name(params){…}` or a
    /// `[static] name [= init];` field.
    fn parse_class_member(&mut self) -> Result<ClassMember, String> {
        let is_static = self.is_kw("static") && !self.peek_is_member_punct(1) && {
            self.advance();
            true
        };
        let start = self.start_at(self.pos);
        // `static { … }` — a class static initialization block (ES2022). A brace
        // where a member key would be is unambiguous: no member name can start
        // with `{`, so this is checked before the key parse (which otherwise
        // rejects it as `bad member key Punct("{")`).
        if is_static && self.is_punct("{") {
            self.advance();
            // Its own function context: `yield`/`await` are plain identifiers
            // inside a static block, whatever encloses the class.
            let body = self.parse_body_block(false, false, false)?;
            return Ok(ClassMember {
                key: Expr::Str(String::new()),
                computed: false,
                kind: MemberKind::StaticBlock,
                is_static: true,
                is_generator: false,
                is_async: false,
                params: Vec::new(),
                body,
                field_init: None,
                span: self.span_from(start),
            });
        }
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
                span: (0, 0),
            });
        }
        // A method / accessor / constructor.
        let is_ctor = !is_static
            && !computed
            && matches!(&key, Expr::Str(s) if s == "constructor")
            && kind == MemberKind::Method;
        let params = self.parse_params()?;
        check_accessor_arity(kind, &params)?;
        self.check_params(&params, self.leading_use_strict(self.pos + 1), true)?;
        self.expect_punct("{")?;
        let body = self.parse_fn_body_block(is_generator, is_async)?;
        Ok(ClassMember {
            key,
            computed,
            kind: if is_ctor {
                MemberKind::Constructor
            } else {
                kind
            },
            is_static,
            is_generator,
            is_async,
            params,
            body,
            field_init: None,
            span: self.span_from(start),
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
                _ => Err(self.unexpected()),
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

    /// Early errors of a `var`/`let`/`const` statement's declarator list: a
    /// `const` or a destructuring pattern needs an initialiser, `let` may not
    /// be a lexically bound name, and `enum` is never a binding.
    fn check_declarators(&self, kind: &DeclKind, decls: &[Declarator]) -> Result<(), String> {
        for d in decls {
            if d.init.is_none() {
                if !matches!(d.target, Expr::Ident(_)) {
                    return Err(
                        "SyntaxError: Missing initializer in destructuring declaration".to_string(),
                    );
                }
                if matches!(kind, DeclKind::Const) {
                    return Err("SyntaxError: Missing initializer in const declaration".to_string());
                }
            }
            if !matches!(kind, DeclKind::Var) {
                if let Expr::Ident(n) = &d.target {
                    if n == "let" {
                        return Err(
                            "SyntaxError: let is disallowed as a lexically bound name".to_string()
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// A statement in a single-statement position — the body of a loop or the
    /// arm of an `if`. Declarations are not statements there (14.6, 14.7);
    /// V8 words the refusal by what the declaration was and by strictness.
    /// Sloppy code keeps B.3.3's one allowance: a plain function declaration
    /// as an `if` arm.
    fn parse_sub_stmt(&mut self, is_if_arm: bool) -> Result<Stmt, String> {
        let is_async_fn =
            self.is_kw("async") && self.peek_kw(1, "function") && !self.peek_newline(1);
        if self.is_kw("function") || is_async_fn {
            let generator = self.peek_punct_after_function();
            if is_async_fn {
                return Err("SyntaxError: Async functions can only be declared at the top level or inside a block.".to_string());
            }
            if generator {
                return Err("SyntaxError: Generators can only be declared at the top level or inside a block.".to_string());
            }
            if self.strict {
                return Err("SyntaxError: In strict mode code, functions can only be declared at top level or inside a block.".to_string());
            }
            if !is_if_arm {
                return Err("SyntaxError: In non-strict mode code, functions can only be declared at top level, inside a block, or as the body of an if statement.".to_string());
            }
        }
        if self.is_kw("class") {
            return Err(self.unexpected());
        }
        if self.is_kw("const") {
            return Err(self.unexpected());
        }
        if self.is_kw("let")
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Punct(p)) if p == "[" || p == "{"
            )
        {
            return Err(
                "SyntaxError: Lexical declaration cannot appear in a single-statement context"
                    .to_string(),
            );
        }
        self.parse_stmt()
    }

    /// Whether the `function` keyword at the cursor is `function*`.
    fn peek_punct_after_function(&self) -> bool {
        let at = if self.is_kw("async") { 2 } else { 1 };
        matches!(self.toks.get(self.pos + at).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "*")
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
        if self.is_punct("[") || self.is_punct("{") {
            let outer = std::mem::replace(&mut self.in_binding, true);
            let pattern = if self.is_punct("[") {
                self.parse_array_literal()
            } else {
                self.parse_object_literal()
            };
            self.in_binding = outer;
            pattern
        } else {
            let name = self.ident_name()?;
            self.check_binding_name(&name)?;
            Ok(Expr::Ident(name))
        }
    }

    /// Words that may not be bound: `enum` always, and the future-reserved
    /// words (plus `let`, `static`, `yield`) in strict code (13.1.1).
    fn check_binding_name(&self, name: &str) -> Result<(), String> {
        const STRICT_RESERVED: &[&str] = &[
            "implements",
            "interface",
            "let",
            "package",
            "private",
            "protected",
            "public",
            "static",
            "yield",
        ];
        if name == "enum" {
            return Err("SyntaxError: Unexpected reserved word".to_string());
        }
        if self.strict && STRICT_RESERVED.contains(&name) {
            return Err("SyntaxError: Unexpected strict mode reserved word".to_string());
        }
        Ok(())
    }

    fn parse_if(&mut self) -> Result<StmtKind, String> {
        self.advance(); // if
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        let cons = Box::new(self.parse_sub_stmt(true)?);
        let alt = if self.eat_kw("else") {
            Some(Box::new(self.parse_sub_stmt(true)?))
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
        let body = Box::new(self.parse_sub_stmt(false)?);
        Ok(StmtKind::While { test, body })
    }

    fn parse_with(&mut self) -> Result<StmtKind, String> {
        if self.strict {
            return Err(
                "SyntaxError: Strict mode code may not include a with statement".to_string(),
            );
        }
        self.advance();
        self.expect_punct("(")?;
        let object = self.parse_expr()?;
        self.expect_punct(")")?;
        let body = Box::new(self.parse_sub_stmt(false)?);
        Ok(StmtKind::With { object, body })
    }

    fn parse_do_while(&mut self) -> Result<StmtKind, String> {
        self.advance();
        let body = Box::new(self.parse_sub_stmt(false)?);
        if !self.eat_kw("while") {
            return Err(self.unexpected());
        }
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        // 12.10.1 rule 3: a `;` is inserted after `do … while ( … )` even with no
        // line break, so `do ; while (0) 1` is two statements.
        self.eat_punct(";");
        Ok(StmtKind::DoWhile { body, test })
    }

    fn parse_for(&mut self) -> Result<StmtKind, String> {
        self.advance();
        // `for await (… of …)` — the async-iteration form (valid in an async body).
        let is_await = self.eat_kw("await");
        self.expect_punct("(")?;
        // Optional declaration or expression init.
        let decl_kind = match self.tok() {
            Tok::Ident(s) if s == "var" || s == "let" || s == "const" => {
                Some(self.parse_decl_kind())
            }
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
        if self.is_kw("of") || self.is_kw("in") {
            // `for ({ a = 1 } of xs)`: the head is a pattern.
            self.cover_init.clear();
        }
        if self.eat_kw("of") {
            let iter = self.parse_assign()?;
            self.expect_punct(")")?;
            let body = Box::new(self.parse_sub_stmt(false)?);
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
            let body = Box::new(self.parse_sub_stmt(false)?);
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
            let mut decls = vec![Declarator {
                target: first_target,
                init,
            }];
            while self.eat_punct(",") {
                let target = self.parse_binding_target()?;
                let init = if self.eat_punct("=") {
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                decls.push(Declarator { target, init });
            }
            // `for (let a, b of …)` / `for (let a = 1 of …)` are for-of headers
            // that carry more than a bare binding.
            if self.is_kw("of") {
                return Err(if decls.len() > 1 {
                    "SyntaxError: Invalid left-hand side in for-of loop: Must have a single binding."
                } else {
                    "SyntaxError: for-of loop variable declaration may not have an initializer."
                }
                .to_string());
            }
            StmtKind::Decl { kind: k, decls }
        } else {
            // A non-declaration C-style init may be a comma sequence
            // (`for (i = 0, n = a.length; …)`) — extend past the first assignment.
            let init = if self.is_punct(",") {
                let mut items = vec![first_target];
                while self.eat_punct(",") {
                    items.push(self.parse_expr_no_in()?);
                }
                Expr::Sequence(items)
            } else {
                first_target
            };
            StmtKind::Expr(init)
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
        let body = Box::new(self.parse_sub_stmt(false)?);
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
                return Err(self.unexpected());
            };
            self.expect_punct(":")?;
            let mut body = Vec::new();
            while !self.is_punct("}")
                && !self.is_kw("case")
                && !self.is_kw("default")
                && !self.at_eof()
            {
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
        let saved = self.no_in;
        self.no_in = true;
        let r = self.parse_assign();
        self.no_in = saved;
        r
    }

    /// Whether `e` is an optional chain — i.e. its member/call SPINE carries a
    /// `?.` link. Mirrors the compiler's spine walk; used only to decide
    /// whether a set of parentheses is a chain boundary worth recording.
    fn has_optional_link(e: &Expr) -> bool {
        match e {
            Expr::Member {
                object, optional, ..
            } => *optional || Self::has_optional_link(object),
            Expr::Index {
                object, optional, ..
            } => *optional || Self::has_optional_link(object),
            Expr::Call { func, optional, .. } => *optional || Self::has_optional_link(func),
            _ => false,
        }
    }

    /// Run `f` with `in` re-enabled (inside a parenthesised/bracketed sub-
    /// expression of a `for` LHS, where the no-in restriction does not apply).
    fn allow_in<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let saved = self.no_in;
        self.no_in = false;
        let r = f(self);
        self.no_in = saved;
        r
    }

    /// `yield [no LineTerminator here] [*] [expr]` (15.5): a YieldExpression is
    /// an AssignmentExpression, so nothing binds to it from the right — `yield
    /// * 2` is a bare `yield` and then a stray `*`.
    fn parse_yield(&mut self) -> Result<Expr, String> {
        self.advance(); // yield
        let delegate = !self.newline_before() && self.eat_punct("*");
        // `yield` with no argument (before `)`, `]`, `}`, `,`, `;`, `:`,
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

    fn parse_assign(&mut self) -> Result<Expr, String> {
        if self.in_generator && self.is_kw("yield") {
            return self.parse_yield();
        }
        // Arrow function detection.
        if let Some(arrow) = self.try_parse_arrow()? {
            return Ok(arrow);
        }
        let start = self.pos;
        let left = self.parse_conditional()?;
        // Assignment operators (right-associative).
        let op = match self.tok() {
            Tok::Punct(p) => p.clone(),
            _ => return Ok(left),
        };
        // `{ a = 1 } = v`: the shorthand initialisers just parsed belong to a
        // pattern, so they are defaults and not an error.
        if op == "=" {
            // A parenthesised literal is a value, never a pattern (13.15.1):
            // `({ a }) = 1` and `([a]) = 1` are early errors, where `(a) = 1` is
            // fine.
            if matches!(left, Expr::Object(_) | Expr::Array(_))
                && matches!(&self.toks[start].tok, Tok::Punct(p) if p == "(")
                && self.matching_paren(start) == Some(self.pos - 1)
            {
                return Err("SyntaxError: Invalid left-hand side in assignment".to_string());
            }
            self.cover_init.retain(|&at| at < start);
        }
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
                // Logical assignment. The operator is CARRIED, not desugared:
                // building `Logical(op, left.clone(), value)` here duplicated
                // the target, so `o[k()] ||= 1` evaluated `k` twice — once for
                // the read and once for the write — where node evaluates it
                // once and may not write at all.
                self.advance();
                let value = self.parse_assign()?;
                let lop = match op.as_str() {
                    "&&=" => LogicalOp::And,
                    "||=" => LogicalOp::Or,
                    _ => LogicalOp::Nullish,
                };
                return Ok(Expr::Assign {
                    target: Box::new(left),
                    op: Some(AssignOp::Logical(lop)),
                    value: Box::new(value),
                });
            }
            _ => return Ok(left),
        };
        self.advance();
        let value = self.parse_assign()?;
        Ok(Expr::Assign {
            target: Box::new(left),
            op: compound.map(AssignOp::Binary),
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
        // The logical operator that built `left` in THIS loop. A parenthesised
        // operand arrives from `parse_unary` and is None, which is what makes
        // `(a || b) ?? c` legal while `a || b ?? c` is not (13.12).
        let mut left_logical: Option<LogicalOp> = None;
        while let Some((prec, right_assoc, logical, bin)) = self.bin_info() {
            if prec < min_prec {
                break;
            }
            if let Some(lop) = logical {
                let nullish = |o: LogicalOp| matches!(o, LogicalOp::Nullish);
                if left_logical.is_some_and(|l| nullish(l) != nullish(lop)) {
                    return Err(self.unexpected());
                }
            }
            self.advance();
            // `??` takes BitwiseOR-level operands (prec 4), never `||`/`&&`.
            let next_min = match logical {
                Some(LogicalOp::Nullish) => 4,
                _ if right_assoc => prec,
                _ => prec + 1,
            };
            let right = self.parse_binary(next_min)?;
            left_logical = logical;
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
            // In a `for` LHS (no-in) context, `in` is the loop separator, not a
            // relational operator.
            Tok::Ident(s) if s == "in" => {
                if self.no_in {
                    return None;
                }
                "in"
            }
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

    /// Reject a `**` directly after a just-parsed UnaryExpression. JS only
    /// allows an UpdateExpression there (`x++ ** y` and `++x ** y` are fine),
    /// so an unparenthesized `-x ** y` / `typeof x ** y` / `await x ** y` is a
    /// SyntaxError rather than a silently-reassociated `-(x ** y)`.
    fn reject_unary_before_pow(&mut self) -> Result<(), String> {
        if self.is_punct("**") {
            return Err("SyntaxError: Unary operator used immediately before \
                 exponentiation expression. Parenthesis must be used to \
                 disambiguate operator precedence"
                .to_string());
        }
        Ok(())
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
            // `ExponentiationExpression : UpdateExpression ** …` — a
            // UnaryExpression on the left of `**` is a SyntaxError, so
            // `-x ** y` must be written `(-x) ** y` or `-(x ** y)`.
            self.reject_unary_before_pow()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        // Prefix ++/--.
        if self.is_punct("++") || self.is_punct("--") {
            let op = if self.is_punct("++") {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
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
            let op = if self.is_punct("++") {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
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
                    return Err(self.unexpected());
                }
                Expr::NewTarget
            } else {
                let callee = self.parse_call_member_no_call()?;
                // An optional chain cannot be the callee of `new` (13.3.5.1):
                // `new a?.b()` and `new C?.()` are early errors. Parenthesising
                // the chain ends it, so `new (a?.b)()` is legal — which is why
                // the check is on the callee's own spine, and the parenthesised
                // form records a chain boundary that clears `optional`.
                if Self::has_optional_link(&callee) || self.is_punct("?.") {
                    return Err(
                        "SyntaxError: Invalid optional chain from new expression".to_string()
                    );
                }
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
                self.check_private_in_scope(&property)?;
                e = Expr::Member {
                    object: Box::new(e),
                    property,
                    optional: false,
                };
            } else if self.eat_punct("?.") {
                if matches!(self.tok(), Tok::Template { .. }) {
                    return Err(
                        "SyntaxError: Invalid tagged template on optional chain".to_string()
                    );
                }
                if self.is_punct("(") {
                    let args = self.parse_args()?;
                    e = Expr::Call {
                        func: Box::new(e),
                        args,
                        optional: true,
                    };
                } else if self.is_punct("[") {
                    self.advance();
                    let index = self.allow_in(|p| p.parse_expr())?;
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
                let index = self.allow_in(|p| p.parse_expr())?;
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
                // A tagged template cannot sit on an optional chain (13.3.11.1):
                // `a?.b`t`` is an early error, because the tag would have to be
                // called even when the chain short-circuited.
                if Self::has_optional_link(&e) {
                    return Err(
                        "SyntaxError: Invalid tagged template on optional chain".to_string()
                    );
                }
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
        let (quasis, raws, exprs_src, expr_at) = match self.tok().clone() {
            Tok::Template {
                quasis,
                raws,
                exprs,
                expr_at,
            } => (quasis, raws, exprs, expr_at),
            _ => unreachable!(),
        };
        self.advance();
        let mut exprs = Vec::new();
        for (src, at) in exprs_src.iter().zip(expr_at) {
            exprs.push(self.parse_field(src, at)?);
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
                self.check_private_in_scope(&property)?;
                e = Expr::Member {
                    object: Box::new(e),
                    property,
                    optional: false,
                };
            } else if self.is_punct("[") {
                self.advance();
                let index = self.allow_in(|p| p.parse_expr())?;
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
        // Inside a call-argument list `in` is always a relational operator, even
        // in a `for` LHS.
        let args = self.allow_in(|p| {
            let mut args = Vec::new();
            while !p.is_punct(")") {
                if p.eat_punct("...") {
                    let e = p.parse_assign()?;
                    args.push(Expr::Spread(Box::new(e)));
                } else {
                    args.push(p.parse_assign()?);
                }
                if !p.eat_punct(",") {
                    break;
                }
            }
            Ok(args)
        })?;
        // V8 reserves its own wording for a call whose argument list does not
        // close, whatever token stopped it (`f(1 2)`, `f(`).
        if !self.eat_punct(")") {
            return Err("SyntaxError: missing ) after argument list".to_string());
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.tok().clone() {
            Tok::Num(n) => {
                self.check_legacy()?;
                self.advance();
                Ok(Expr::Number(n))
            }
            Tok::BigInt(s) => {
                self.advance();
                Ok(Expr::BigInt(s))
            }
            Tok::Regex(pat, flags) => {
                // A regex LITERAL's flags and pattern are early errors (13.2.7.3),
                // reported before anything in the program runs.
                crate::regexp::check_literal(&pat, &flags)?;
                self.advance();
                Ok(Expr::Regex(pat, flags))
            }
            Tok::Str(s) => {
                self.check_legacy()?;
                self.advance();
                Ok(Expr::Str(s))
            }
            Tok::Template {
                quasis,
                raws: _,
                exprs,
                expr_at,
            } => {
                self.advance();
                let mut parsed = Vec::new();
                for (src, at) in exprs.iter().zip(expr_at) {
                    parsed.push(self.parse_field(src, at)?);
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
                // Parentheses are otherwise erased, but around an OPTIONAL CHAIN
                // they are load-bearing: they END the chain, so a `?.` inside
                // them cannot short-circuit an access written outside them.
                // `o?.a.b` is `undefined` when `o` is nullish; `(o?.a).b` reads
                // `.b` off that `undefined` and THROWS (ECMA-262 13.3.1 —
                // `ParenthesizedExpression` is not an `OptionalExpression`).
                // With the parentheses dropped the two parsed to the same tree
                // and both answered `undefined`.
                //
                // The boundary is spelled as a one-element sequence rather than
                // a new node type: `(e)` and `e,` evaluate identically, every
                // existing pass already walks `Sequence` correctly, and the
                // chain-spine walk does not descend through it — which is the
                // whole point. Only chains are wrapped, so no other expression's
                // tree shape changes.
                if Self::has_optional_link(&e) {
                    return Ok(Expr::Sequence(vec![e]));
                }
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
                        if !self.super_ok {
                            return Err("SyntaxError: 'super' keyword unexpected here".to_string());
                        }
                        self.advance();
                        Ok(Expr::Super)
                    }
                    "class" => Ok(Expr::Class(Box::new(self.parse_class(false)?))),
                    "function" => self.parse_function_expr(false),
                    "async" if self.peek_kw(1, "function") && !self.peek_newline(1) => {
                        self.advance(); // async
                        self.parse_function_expr(true)
                    }
                    // `yield` in a generator is an AssignmentExpression and is
                    // parsed by `parse_assign`; reaching it here means it sat
                    // where only a higher-precedence operand may (`a + yield`).
                    "yield" if self.in_generator => Err(self.unexpected()),
                    "await" if self.in_async => {
                        self.advance();
                        let e = self.parse_unary()?;
                        // An AwaitExpression is a UnaryExpression, so it too
                        // cannot sit directly left of `**`.
                        self.reject_unary_before_pow()?;
                        Ok(Expr::Await(Box::new(e)))
                    }
                    _ if is_keyword(&s) => Err(self.unexpected()),
                    // The future-reserved words are not identifiers in strict code.
                    "implements" | "interface" | "package" | "private" | "protected" | "public"
                    | "static" | "yield"
                        if self.strict =>
                    {
                        Err("SyntaxError: Unexpected strict mode reserved word".to_string())
                    }
                    _ => {
                        self.advance();
                        Ok(Expr::Ident(s))
                    }
                }
            }
            _ => Err(self.unexpected()),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, String> {
        self.expect_punct("[")?;
        let mut items = Vec::new();
        while !self.is_punct("]") {
            if self.is_punct(",") {
                // Elision: the element is a HOLE, not a stored `undefined`.
                items.push(Expr::Hole);
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
        let mut proto_seen = false;
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
            if (self.is_kw("get") || self.is_kw("set"))
                && !self.peek_is_member_punct(1)
                && !matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == ":" || p == ",")
            {
                let is_getter = self.is_kw("get");
                let start = self.start_at(self.pos);
                self.advance();
                let (key, computed) = self.parse_property_key()?;
                let params = self.parse_params()?;
                check_accessor_arity(
                    if is_getter {
                        MemberKind::Get
                    } else {
                        MemberKind::Set
                    },
                    &params,
                )?;
                self.check_params(&params, self.leading_use_strict(self.pos + 1), true)?;
                self.expect_punct("{")?;
                let body = self.with_super(true, |p| p.parse_fn_body_block(false, false))?;
                let func = Expr::Function {
                    params,
                    body: FnBody::Block(body),
                    is_arrow: false,
                    name: None,
                    is_generator: false,
                    is_async: false,
                    is_method: true,
                    span: self.span_from(start),
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
            let start = self.start_at(self.pos);
            if self.is_kw("async")
                && !self.peek_is_member_punct(1)
                && !self.peek_newline(1)
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
                self.check_params(&params, self.leading_use_strict(self.pos + 1), true)?;
                self.expect_punct("{")?;
                let body = self.with_super(true, |p| p.parse_fn_body_block(m_gen, m_async))?;
                let f = Expr::Function {
                    params,
                    body: FnBody::Block(body),
                    is_arrow: false,
                    name: None,
                    is_generator: m_gen,
                    is_async: m_async,
                    is_method: true,
                    span: self.span_from(start),
                };
                props.push(Prop::KeyValue {
                    key,
                    value: f,
                    computed,
                });
            } else if self.eat_punct(":") {
                // 13.2.5.1: a second `__proto__: v` in one literal is an early error
                // (the shorthand and method forms do not set the prototype).
                if !computed && matches!(&key, Expr::Str(k) if k == "__proto__") {
                    if proto_seen {
                        return Err(
                            "SyntaxError: Duplicate __proto__ fields are not allowed in object literals"
                                .to_string(),
                        );
                    }
                    proto_seen = true;
                }
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
                let value = if self.is_punct("=") {
                    if !self.in_binding {
                        self.cover_init.push(self.pos);
                    }
                    self.advance();
                    // Pattern default; represent as Assign so destructuring reads it.
                    let d = self.parse_assign()?;
                    Expr::Assign {
                        target: Box::new(Expr::Ident(name.clone())),
                        op: None,
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
        let start = self.start_at(self.pos);
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
            if !is_keyword(name)
                && matches!(self.toks.get(base + 1).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "=>")
            {
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
                    is_method: false,
                    span: self.span_from(start),
                }));
            }
        }
        // `( ... ) => ...`
        if matches!(self.toks.get(base).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "(") {
            if let Some(close) = self.matching_paren(base) {
                let after = close + 1;
                if matches!(self.toks.get(after).map(|t| &t.tok), Some(Tok::Punct(p)) if p == "=>")
                {
                    if is_async {
                        self.advance(); // async
                    }
                    let params = self.parse_params()?;
                    self.check_params(&params, false, true)?;
                    self.expect_punct("=>")?;
                    let body = self.parse_arrow_body(is_async)?;
                    return Ok(Some(Expr::Function {
                        params,
                        body,
                        is_arrow: true,
                        name: None,
                        is_generator: false,
                        is_async,
                        is_method: false,
                        span: self.span_from(start),
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
        let outer_labels = std::mem::take(&mut self.labels);
        let outer_strict = self.strict;
        let outer_in_function = std::mem::replace(&mut self.in_function, true);
        let r = if self.is_punct("{") {
            self.advance();
            self.strict = outer_strict || self.leading_use_strict(self.pos);
            self.parse_block_body().map(FnBody::Block)
        } else {
            self.parse_assign().map(|e| FnBody::Expr(Box::new(e)))
        };
        self.in_function = outer_in_function;
        self.strict = outer_strict;
        self.labels = outer_labels;
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

/// A parser over a template-literal `${...}` field's raw source. `base` is the
/// field's byte offset in the enclosing script, so the spans recorded inside it
/// index that script; `None` records none.
fn field_parser(src: &str, base: Option<u32>) -> Result<Parser, String> {
    let mut toks = lex(src)?;
    for t in &mut toks {
        t.start += base.unwrap_or(0);
        t.end += base.unwrap_or(0);
        if let Tok::Template { expr_at, .. } = &mut t.tok {
            for at in expr_at {
                *at += base.unwrap_or(0);
            }
        }
    }
    Ok(Parser {
        toks,
        pos: 0,
        in_generator: false,
        in_async: false,
        no_in: false,
        class_scopes: Vec::new(),
        labels: Vec::new(),
        strict: false,
        super_ok: false,
        in_function: true,
        cover_init: Vec::new(),
        in_binding: false,
        spans: base.is_some(),
    })
}
