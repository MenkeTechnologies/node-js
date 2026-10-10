//! JavaScript tokenizer.
//!
//! Produces a flat token stream ending in `Eof`. Unlike Python, JS is not
//! indentation-sensitive: blocks are brace-delimited and statements are
//! semicolon-terminated, with Automatic Semicolon Insertion (ASI) filling in
//! for newline-terminated statements. Each token records whether a line break
//! preceded it (`newline_before`) so the parser can apply ASI. `//` and `/* */`
//! comments are stripped here. Template literals are emitted as a single
//! `Template` token carrying the cooked quasis plus the raw source of each
//! `${...}` field; the parser recursively parses those fields.

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    /// A `BigInt` literal (`10n`, `0xffn`, …) carried as its canonical decimal
    /// digit string; the compiler lowers it to a heap `JsObj::BigInt`.
    BigInt(String),
    /// A regular-expression literal (`/pat/flags`): `(pattern, flags)`. The lexer
    /// only recognizes it in expression-start position (see `regex_allowed`).
    Regex(String, String),
    Str(String),
    /// A template literal: `quasis.len() == exprs.len() + 1`. `quasis` are the
    /// cooked (escape-decoded) strings, `raws` the corresponding raw source
    /// (undecoded, for tagged templates / `String.raw`), and each `exprs` entry is
    /// the raw source text between `${` and its matching `}`.
    Template {
        quasis: Vec<String>,
        raws: Vec<String>,
        exprs: Vec<String>,
        /// Byte offset of each `exprs` entry in the text handed to [`lex`].
        expr_at: Vec<u32>,
    },
    Ident(String),
    /// An operator or delimiter, e.g. `+`, `===`, `=>`, `(`, `{`, `.`, `?.`.
    Punct(String),
    Eof,
}

/// A token plus its 1-based source line and whether a newline preceded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub newline_before: bool,
    /// UTF-8 byte offsets of the token's first character and one past its
    /// last, in the text handed to [`lex`].
    pub start: u32,
    pub end: u32,
    /// A legacy form this token spelled, which only STRICT code rejects — and
    /// strictness is not known until the parser has seen the directive
    /// prologue, so the lexer records the fact and the parser decides.
    pub legacy: Legacy,
}

/// The strict-mode-only restrictions a token can trip (B.1.1, B.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Legacy {
    None,
    /// `010`: a legacy octal integer literal.
    OctalLiteral,
    /// `08`, `09.5`: a decimal literal with a leading zero.
    LeadingZeroDecimal,
    /// `"\101"`, `"\01"`: a legacy octal escape in a string.
    OctalEscape,
    /// `"\8"`, `"\9"`.
    NonOctalEscape,
}

struct Lexer {
    src: Vec<char>,
    pos: usize,
    /// Byte offset of each char index (`src.len() + 1` entries).
    byte_at: Vec<u32>,
    /// Char index where the token being scanned began.
    tok_start: usize,
    line: u32,
    out: Vec<Token>,
    pending_newline: bool,
    /// The legacy form the token being scanned has used so far; moved into
    /// the token by `push`.
    legacy: Legacy,
}

/// Multi-char operators, longest first so the scanner is greedy.
const OPS4: &[&str] = &[">>>="];
const OPS3: &[&str] = &[
    "===", "!==", "**=", "...", ">>>", "<<=", ">>=", "&&=", "||=", "??=",
];
const OPS2: &[&str] = &[
    "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "=>", "++", "--", "+=", "-=", "*=", "/=", "%=",
    "&=", "|=", "^=", "<<", ">>", "**",
];

/// Tokenize `src` into a token stream ending in `Eof`.
pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let mut byte_at: Vec<u32> = src.char_indices().map(|(b, _)| b as u32).collect();
    byte_at.push(src.len() as u32);
    let mut lx = Lexer {
        src: src.chars().collect(),
        pos: 0,
        byte_at,
        tok_start: 0,
        line: 1,
        out: Vec::new(),
        pending_newline: false,
        legacy: Legacy::None,
    };
    lx.run()?;
    Ok(lx.out)
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.src.get(self.pos + n).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.src.get(self.pos).copied();
        if let Some(ch) = c {
            self.pos += 1;
            if ch == '\n' {
                self.line += 1;
            }
        }
        c
    }
    fn push(&mut self, tok: Tok) {
        self.out.push(Token {
            tok,
            line: self.line,
            newline_before: self.pending_newline,
            start: self.byte_at[self.tok_start.min(self.pos)],
            end: self.byte_at[self.pos],
            legacy: std::mem::replace(&mut self.legacy, Legacy::None),
        });
        self.pending_newline = false;
    }

    fn run(&mut self) -> Result<(), String> {
        // A Hashbang comment (12.5) is legal only as the very first thing in
        // the source: `#!/usr/bin/env node` runs to the end of its line.
        if self.peek() == Some('#') && self.peek_at(1) == Some('!') {
            while !matches!(self.peek(), None | Some('\n')) {
                self.bump();
            }
        }
        while self.lex_step()? {}
        self.tok_start = self.pos;
        self.push(Tok::Eof);
        Ok(())
    }

    /// Consume one whitespace run, comment or token. `false` at end of input.
    fn lex_step(&mut self) -> Result<bool, String> {
        {
            match self.peek() {
                None => return Ok(false),
                Some('\n') => {
                    self.bump();
                    self.pending_newline = true;
                }
                // CR, LS and PS are line terminators (12.3) just as LF is.
                Some('\r' | '\u{2028}' | '\u{2029}') => {
                    self.bump();
                    self.pending_newline = true;
                }
                // WhiteSpace (12.2): TAB, VT, FF, the BOM and every Zs space.
                Some(
                    ' '
                    | '\t'
                    | '\u{0B}'
                    | '\u{0C}'
                    | '\u{FEFF}'
                    | '\u{A0}'
                    | '\u{1680}'
                    | '\u{2000}'..='\u{200A}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}',
                ) => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    self.skip_line_comment();
                }
                // B.1.1 HTML-like comments: `<!--` opens a single-line comment
                // anywhere, `-->` only where a line begins (start of input, or
                // after a line terminator).
                Some('<')
                    if self.peek_at(1) == Some('!')
                        && self.peek_at(2) == Some('-')
                        && self.peek_at(3) == Some('-') =>
                {
                    self.skip_line_comment();
                }
                Some('-')
                    if (self.pending_newline || self.out.is_empty())
                        && self.peek_at(1) == Some('-')
                        && self.peek_at(2) == Some('>') =>
                {
                    self.skip_line_comment();
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    let mut closed = false;
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek_at(1) == Some('/') {
                            self.bump();
                            self.bump();
                            closed = true;
                            break;
                        }
                        if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                            self.pending_newline = true;
                        }
                        self.bump();
                    }
                    if !closed {
                        return Err("SyntaxError: Invalid or unexpected token".to_string());
                    }
                }
                // A `/` in expression-start position is a regex literal, not the
                // division operator (comments were already ruled out above).
                Some('/') if self.regex_allowed() => {
                    self.tok_start = self.pos;
                    self.scan_regex()?
                }
                Some(_) => {
                    self.tok_start = self.pos;
                    self.scan_token()?
                }
            }
        }
        Ok(true)
    }

    /// Consume to the end of a single-line comment, stopping AT the line
    /// terminator (the main loop then records the newline). All four
    /// LineTerminators end one — not only `\n`.
    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                break;
            }
            self.bump();
        }
    }

    /// Whether a `/` here begins a regex literal (expression-start position)
    /// rather than the division operator. Decided by the previous significant
    /// token: after a value (identifier/number/string/`)`/`]`) `/` is division;
    /// after an operator, `(`, `,`, `{`, `[`, `;`, `:`, `return`, etc. it opens a
    /// regex. This is the standard "regex-or-divide" ASI-adjacent heuristic.
    fn regex_allowed(&self) -> bool {
        match self.out.last().map(|t| &t.tok) {
            None => true, // program start
            Some(Tok::Num(_))
            | Some(Tok::BigInt(_))
            | Some(Tok::Str(_))
            | Some(Tok::Template { .. })
            | Some(Tok::Regex(..)) => false,
            Some(Tok::Ident(s)) => matches!(
                s.as_str(),
                // Keywords that precede an expression → regex; a plain variable
                // name (or a value keyword like `this`/`true`) → division.
                "return"
                    | "typeof"
                    | "instanceof"
                    | "in"
                    | "of"
                    | "new"
                    | "delete"
                    | "void"
                    | "do"
                    | "else"
                    | "case"
                    | "throw"
                    | "yield"
                    | "await"
            ),
            Some(Tok::Punct(p)) => !matches!(p.as_str(), ")" | "]" | "}" | "++" | "--"),
            Some(Tok::Eof) => true,
        }
    }

    /// Scan a `/pat/flags` regex literal. The opening `/` is current. The body
    /// runs to the next unescaped `/` that is not inside a `[...]` character
    /// class; trailing ASCII-letter flags follow.
    fn scan_regex(&mut self) -> Result<(), String> {
        self.bump(); // opening slash
        let mut pat = String::new();
        let mut in_class = false;
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err("SyntaxError: Invalid regular expression: missing /".to_string())
                }
                Some('\\') => {
                    // Keep the escape verbatim (the translator interprets it).
                    pat.push('\\');
                    self.bump();
                    if let Some(c) = self.bump() {
                        pat.push(c);
                    }
                }
                Some('[') => {
                    in_class = true;
                    pat.push('[');
                    self.bump();
                }
                Some(']') => {
                    in_class = false;
                    pat.push(']');
                    self.bump();
                }
                Some('/') if !in_class => {
                    self.bump();
                    break;
                }
                Some(c) => {
                    pat.push(c);
                    self.bump();
                }
            }
        }
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                flags.push(c);
                self.bump();
            } else {
                break;
            }
        }
        self.push(Tok::Regex(pat, flags));
        Ok(())
    }

    fn scan_token(&mut self) -> Result<(), String> {
        let c = self.peek().unwrap();
        if c == '"' || c == '\'' {
            return self.scan_string(c);
        }
        if c == '`' {
            return self.scan_template();
        }
        // IdentifierStart (12.7) is any Unicode letter, not only ASCII:
        // `const é = 1` and `let Δx` are ordinary names. `scan_name` already
        // continues on any alphanumeric.
        if c.is_alphabetic() || c == '_' || c == '$' || (c == '\\' && self.peek_at(1) == Some('u'))
        {
            return self.scan_name();
        }
        // Private class member (`#name`): scanned as an identifier keeping the `#`.
        if c == '#'
            && self
                .peek_at(1)
                .map(|d| d.is_alphabetic() || d == '_' || d == '$')
                .unwrap_or(false)
        {
            return self.scan_name();
        }
        if c.is_ascii_digit()
            || (c == '.' && self.peek_at(1).map(|d| d.is_ascii_digit()).unwrap_or(false))
        {
            return self.scan_number();
        }
        self.scan_op()
    }

    fn scan_name(&mut self) -> Result<(), String> {
        let mut s = String::new();
        // A leading `#` (private class member name) is kept as part of the ident.
        if self.peek() == Some('#') {
            s.push('#');
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                s.push(c);
                self.pos += 1;
            } else if c == '\\' && self.peek_at(1) == Some('u') {
                // A `\uXXXX` / `\u{…}` escape spells one identifier character
                // (12.7); the name it forms is the decoded one.
                self.pos += 2;
                let mut decoded = String::new();
                push_escape(&mut decoded, 'u', self)?;
                let first = s.is_empty() || s == "#";
                let ok = decoded.chars().next().is_some_and(|d| {
                    d == '_'
                        || d == '$'
                        || if first {
                            d.is_alphabetic()
                        } else {
                            d.is_alphanumeric()
                        }
                });
                if !ok {
                    return Err("SyntaxError: Invalid Unicode escape sequence".to_string());
                }
                s.push_str(&decoded);
            } else {
                break;
            }
        }
        self.push(Tok::Ident(s));
        Ok(())
    }

    fn scan_string(&mut self, quote: char) -> Result<(), String> {
        self.bump(); // opening quote
        let mut raw = String::new();
        loop {
            match self.peek() {
                None => return Err("SyntaxError: Invalid or unexpected token".to_string()),
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    self.bump();
                    if let Some(e) = self.bump() {
                        push_escape(&mut raw, e, self)?;
                    }
                }
                Some('\n') => return Err("SyntaxError: Invalid or unexpected token".to_string()),
                Some(c) => {
                    raw.push(c);
                    self.bump();
                }
            }
        }
        self.push(Tok::Str(raw));
        Ok(())
    }

    /// Lex a template's `${ … }` field to its closing brace and return its
    /// source text. The cursor starts just past `${` and ends just past the
    /// matching `}`; the tokens of the field are scanned (so nested templates,
    /// regex literals and comments are understood) and then discarded, since
    /// the parser re-reads the text.
    fn scan_field(&mut self) -> Result<String, String> {
        let start = self.pos;
        let saved_out = std::mem::take(&mut self.out);
        let saved_newline = std::mem::replace(&mut self.pending_newline, false);
        let mut depth = 0usize;
        let end = loop {
            let before = self.out.len();
            if !self.lex_step()? {
                return Err("SyntaxError: Unexpected end of input".to_string());
            }
            if self.out.len() == before {
                continue;
            }
            match self.out.last().map(|t| &t.tok) {
                Some(Tok::Punct(p)) if p == "{" => depth += 1,
                Some(Tok::Punct(p)) if p == "}" => {
                    if depth == 0 {
                        break self.tok_start;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        };
        self.out = saved_out;
        self.pending_newline = saved_newline;
        Ok(self.src[start..end].iter().collect())
    }

    /// Scan a `` `...${expr}...` `` template. Cooked quasis are decoded; each
    /// `${...}` field's raw source (with balanced braces) is captured for the
    /// parser to re-parse.
    fn scan_template(&mut self) -> Result<(), String> {
        self.bump(); // opening backtick
        let mut quasis = Vec::new();
        let mut raws = Vec::new();
        let mut exprs = Vec::new();
        let mut expr_at = Vec::new();
        let mut cur = String::new();
        let mut cur_raw = String::new();
        loop {
            match self.peek() {
                None => return Err("SyntaxError: Unexpected end of input".to_string()),
                Some('`') => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    // Cooked decodes the escape; raw keeps the exact source span it
                    // spans (including any hex/unicode digits push_escape consumes).
                    let start = self.pos;
                    self.bump();
                    if let Some(e) = self.bump() {
                        let _ = push_escape(&mut cur, e, self);
                    }
                    for c in &self.src[start..self.pos] {
                        cur_raw.push(*c);
                    }
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    self.bump();
                    self.bump();
                    quasis.push(std::mem::take(&mut cur));
                    raws.push(std::mem::take(&mut cur_raw));
                    // The field's extent is found by LEXING it, not by counting
                    // braces and skipping quotes: a regex literal, a comment or a
                    // nested template may each hold a `}` or a quote of their own.
                    expr_at.push(self.byte_at[self.pos]);
                    let src = self.scan_field()?;
                    exprs.push(src);
                }
                Some(c) => {
                    cur.push(c);
                    cur_raw.push(c);
                    self.bump();
                }
            }
        }
        quasis.push(cur);
        raws.push(cur_raw);
        self.push(Tok::Template {
            quasis,
            raws,
            exprs,
            expr_at,
        });
        Ok(())
    }

    /// A run of digits of `radix`, with `_` numeric separators (12.9.4), appended
    /// to `out`. A separator must sit BETWEEN two digits; V8 words each way of
    /// breaking that differently.
    fn read_digits(&mut self, out: &mut String, radix: u32) -> Result<(), String> {
        const INVALID: &str = "SyntaxError: Invalid or unexpected token";
        let mut prev_digit = false;
        while let Some(c) = self.peek() {
            if c.is_digit(radix) {
                out.push(c);
                self.pos += 1;
                prev_digit = true;
            } else if c == '_' {
                if !prev_digit {
                    return Err(
                        if out.is_empty() || !out.ends_with(|d: char| d.is_digit(radix)) {
                            INVALID.to_string()
                        } else {
                            "SyntaxError: Only one underscore is allowed as numeric separator"
                                .to_string()
                        },
                    );
                }
                match self.peek_at(1) {
                    Some(n) if n.is_digit(radix) => {
                        self.pos += 1;
                        prev_digit = false;
                    }
                    Some('_') => {
                        return Err(
                            "SyntaxError: Only one underscore is allowed as numeric separator"
                                .to_string(),
                        )
                    }
                    _ => {
                        return Err("SyntaxError: Numeric separators are not allowed at the \
                                    end of numeric literals"
                            .to_string())
                    }
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    /// 12.9.1: the character right after a NumericLiteral may not start an
    /// identifier or be a digit (`3in x`, `0b12`, `1a`).
    fn reject_trailing_ident(&self) -> Result<(), String> {
        match self.peek() {
            Some(c) if c.is_alphanumeric() || c == '_' || c == '$' || c == '\\' => {
                Err("SyntaxError: Invalid or unexpected token".to_string())
            }
            _ => Ok(()),
        }
    }

    fn scan_number(&mut self) -> Result<(), String> {
        // Radix prefixes: 0x / 0o / 0b.
        if self.peek() == Some('0') {
            if let Some(r) = self.peek_at(1) {
                if matches!(r, 'x' | 'X' | 'o' | 'O' | 'b' | 'B') {
                    self.bump();
                    self.bump();
                    let radix = match r.to_ascii_lowercase() {
                        'x' => 16,
                        'o' => 8,
                        _ => 2,
                    };
                    let mut digits = String::new();
                    self.read_digits(&mut digits, radix)?;
                    if digits.is_empty() {
                        return Err("SyntaxError: Invalid or unexpected token".to_string());
                    }
                    let big = num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)
                        .ok_or_else(|| "SyntaxError: Invalid or unexpected token".to_string())?;
                    // `0x..n` / `0o..n` / `0b..n` BigInt literal: arbitrary
                    // precision, kept as a bignum rather than squeezed through
                    // `i64` (which also refused a plain literal past 2^63).
                    if self.peek() == Some('n') {
                        self.pos += 1;
                        self.reject_trailing_ident()?;
                        self.push(Tok::BigInt(big.to_string()));
                        return Ok(());
                    }
                    self.reject_trailing_ident()?;
                    self.push(Tok::Num(crate::host::bigint_to_f64(&big)));
                    return Ok(());
                }
            }
        }
        // A LEGACY OCTAL literal: `0` followed by digit-run that is all 0-7 is
        // base 8 (`012` is 10, not 12). A run containing an 8 or a 9 is the
        // legacy DECIMAL form and stays base 10 (`08` is 8). Both are
        // SyntaxErrors in strict code, which this lexer cannot see — recorded in
        // BUGS.md. The value was simply read as decimal, so `012` was 12.
        if self.peek() == Some('0') {
            let mut n = 1;
            while self.peek_at(n).is_some_and(|c| c.is_ascii_digit()) {
                n += 1;
            }
            let run: String = (0..n).filter_map(|i| self.peek_at(i)).collect();
            // All-octal digits make it a legacy OCTAL literal whatever follows; what
            // follows is then judged as for any literal (`07e1`, `07n`, `07_` are
            // errors, `07.5` is `07` and a number).
            if n > 1 && run.chars().all(|c| ('0'..='7').contains(&c)) {
                self.pos += n;
                let v = u64::from_str_radix(&run, 8).unwrap_or(0);
                self.reject_trailing_ident()?;
                self.legacy = Legacy::OctalLiteral;
                self.push(Tok::Num(v as f64));
                return Ok(());
            }
            // `0_1`: a separator may not follow the leading zero.
            if self.peek_at(1) == Some('_') {
                return Err(
                    "SyntaxError: Numeric separator can not be used after leading 0.".to_string(),
                );
            }
        }
        // DecimalLiteral (12.9.3): `1`, `1.`, `1.5`, `.5`, each with an optional
        // exponent. Exactly ONE `.` belongs to the literal — `5..toString()` is
        // `5.` then member access, and `1.5.5` is a number followed by `.5`.
        let mut s = String::new();
        let legacy_zero =
            self.peek() == Some('0') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit());
        if legacy_zero {
            self.legacy = Legacy::LeadingZeroDecimal;
            // `08`, `09.5`: a NonOctalDecimalIntegerLiteral takes no separators.
            while let Some(c) = self.peek().filter(char::is_ascii_digit) {
                s.push(c);
                self.pos += 1;
            }
            if self.peek() == Some('_') {
                return Err("SyntaxError: Invalid or unexpected token".to_string());
            }
        } else if self.peek() != Some('.') {
            self.read_digits(&mut s, 10)?;
        }
        let mut is_integer = true;
        if self.peek() == Some('.') {
            is_integer = false;
            s.push('.');
            self.pos += 1;
            self.read_digits(&mut s, 10)?;
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            is_integer = false;
            s.push('e');
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                s.push(self.peek().unwrap());
                self.pos += 1;
            }
            let before = s.len();
            self.read_digits(&mut s, 10)?;
            if s.len() == before {
                return Err("SyntaxError: Invalid or unexpected token".to_string());
            }
        }
        // Decimal `BigInt` literal (`123n`): only an integer digit run may carry
        // the `n` suffix, and `01n` (a legacy-leading-zero run) may not.
        if self.peek() == Some('n') && is_integer && !legacy_zero {
            self.pos += 1;
            self.reject_trailing_ident()?;
            self.push(Tok::BigInt(s));
            return Ok(());
        }
        self.reject_trailing_ident()?;
        let v: f64 = s.parse().map_err(|_| {
            format!(
                "SyntaxError: Invalid or unexpected token (line {})",
                self.line
            )
        })?;
        self.push(Tok::Num(v));
        Ok(())
    }

    fn scan_op(&mut self) -> Result<(), String> {
        let slice: String = self.src[self.pos..(self.pos + 4).min(self.src.len())]
            .iter()
            .collect();
        for op in OPS4 {
            if slice.starts_with(op) {
                self.pos += 4;
                self.push(Tok::Punct((*op).to_string()));
                return Ok(());
            }
        }
        for op in OPS3 {
            if slice.starts_with(op) {
                self.pos += 3;
                self.push(Tok::Punct((*op).to_string()));
                return Ok(());
            }
        }
        for op in OPS2 {
            if slice.starts_with(op) {
                self.pos += 2;
                self.push(Tok::Punct((*op).to_string()));
                return Ok(());
            }
        }
        let c = self.bump().unwrap();
        if "+-*/%<>=!&|^~?:;,.(){}[]".contains(c) {
            self.push(Tok::Punct(c.to_string()));
            Ok(())
        } else {
            Err("SyntaxError: Invalid or unexpected token".to_string())
        }
    }
}

/// Append one escape sequence's decoded character(s) to `out`. `\xNN` and
/// The value of a `\uDC00..\uDFFF` escape sitting at the lexer's cursor, without
/// consuming it. Used to rejoin a surrogate PAIR written as two escapes.
fn peek_low_surrogate(lx: &Lexer) -> Option<u32> {
    if lx.peek() != Some('\\') || lx.peek_at(1) != Some('u') {
        return None;
    }
    let mut n = 0u32;
    for i in 0..4 {
        let c = lx.peek_at(2 + i)?;
        n = n * 16 + c.to_digit(16)?;
    }
    (0xDC00..=0xDFFF).contains(&n).then_some(n)
}

/// Append the code point `n` to a string literal's value.
///
/// `char::from_u32` rejects `U+D800..=U+DFFF`, and the old code simply dropped
/// what it rejected — so `"\ud800".length` was 0 where every engine says 1, and
/// `"a\ud800b".length` was 2 instead of 3. This runtime's documented policy for
/// an unpaired surrogate is to substitute `U+FFFD` (see `utf16`), which is ONE
/// code unit and therefore keeps the length arithmetic exact; dropping the unit
/// broke that invariant rather than implementing it.
fn push_code_point(out: &mut String, n: u32) {
    match char::from_u32(n) {
        Some(ch) => out.push(ch),
        None => out.push('\u{FFFD}'),
    }
}

/// Decode the escape sequence whose introducing char `e` follows a `\` (12.9.4).
///
/// A malformed `\x`, `\u` or `\u{}` is a SyntaxError carrying V8's wording; the
/// template scanner discards it, since an untagged-template error is reported
/// by its own rules and a tagged one cooks to `undefined`. Unknown escapes keep
/// the literal char, which is what NonEscapeCharacter means.
fn push_escape(out: &mut String, e: char, lx: &mut Lexer) -> Result<(), String> {
    const BAD_HEX: &str = "SyntaxError: Invalid hexadecimal escape sequence";
    const BAD_UNICODE: &str = "SyntaxError: Invalid Unicode escape sequence";
    match e {
        'n' => out.push('\n'),
        't' => out.push('\t'),
        'r' => out.push('\r'),
        'b' => out.push('\u{08}'),
        'f' => out.push('\u{0C}'),
        'v' => out.push('\u{0B}'),
        // A LineContinuation contributes nothing; `\r\n` is one terminator.
        '\n' | '\u{2028}' | '\u{2029}' => {}
        '\r' => {
            if lx.peek() == Some('\n') {
                lx.bump();
            }
        }
        // `\0` not followed by a digit is NUL; otherwise this is a LEGACY OCTAL
        // escape (B.1.2): up to three digits when the first is 0-3, else two.
        '0'..='7' => {
            if e == '0' && !lx.peek().is_some_and(|c| c.is_ascii_digit()) {
                out.push('\0');
            } else {
                lx.legacy = Legacy::OctalEscape;
                let mut n = e.to_digit(8).unwrap();
                let max_len = if e <= '3' { 3 } else { 2 };
                for _ in 1..max_len {
                    match lx.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            n = n * 8 + d;
                            lx.bump();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
            }
        }
        'x' => {
            let mut n = 0u32;
            for _ in 0..2 {
                match lx.peek().and_then(|c| c.to_digit(16)) {
                    Some(d) => {
                        n = n * 16 + d;
                        lx.bump();
                    }
                    None => return Err(BAD_HEX.to_string()),
                }
            }
            push_code_point(out, n);
        }
        'u' => {
            if lx.peek() == Some('{') {
                lx.bump();
                let mut n = 0u32;
                let mut digits = 0;
                loop {
                    match lx.peek() {
                        Some('}') if digits > 0 => {
                            lx.bump();
                            break;
                        }
                        Some(c) if c.is_ascii_hexdigit() => {
                            n = n.saturating_mul(16).saturating_add(c.to_digit(16).unwrap());
                            digits += 1;
                            lx.bump();
                        }
                        _ => return Err(BAD_UNICODE.to_string()),
                    }
                    if n > 0x10FFFF {
                        return Err("SyntaxError: Undefined Unicode code-point".to_string());
                    }
                }
                push_code_point(out, n);
            } else {
                let mut n = 0u32;
                for _ in 0..4 {
                    match lx.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            n = n * 16 + d;
                            lx.bump();
                        }
                        None => return Err(BAD_UNICODE.to_string()),
                    }
                }
                // A HIGH surrogate followed by a `\uXXXX` LOW surrogate is one
                // astral character, and `"😀"` is the ordinary
                // ASCII-safe way to write one. Decoding each half on its own
                // turned every such literal into two `U+FFFD`s.
                if (0xD800..=0xDBFF).contains(&n) {
                    if let Some(lo) = peek_low_surrogate(lx) {
                        for _ in 0..6 {
                            lx.bump();
                        }
                        let cp = 0x10000 + ((n - 0xD800) << 10) + (lo - 0xDC00);
                        push_code_point(out, cp);
                        return Ok(());
                    }
                }
                push_code_point(out, n);
            }
        }
        '8' | '9' => {
            lx.legacy = Legacy::NonOctalEscape;
            out.push(e);
        }
        other => out.push(other),
    }
    Ok(())
}
