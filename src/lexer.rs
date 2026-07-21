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
}

struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: u32,
    out: Vec<Token>,
    pending_newline: bool,
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
    let mut lx = Lexer {
        src: src.chars().collect(),
        pos: 0,
        line: 1,
        out: Vec::new(),
        pending_newline: false,
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
        });
        self.pending_newline = false;
    }

    fn run(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                None => break,
                Some('\n') => {
                    self.bump();
                    self.pending_newline = true;
                }
                Some(c) if c == ' ' || c == '\t' || c == '\r' => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek_at(1) == Some('/') {
                            self.bump();
                            self.bump();
                            break;
                        }
                        if c == '\n' {
                            self.pending_newline = true;
                        }
                        self.bump();
                    }
                }
                // A `/` in expression-start position is a regex literal, not the
                // division operator (comments were already ruled out above).
                Some('/') if self.regex_allowed() => self.scan_regex()?,
                Some(_) => self.scan_token()?,
            }
        }
        self.push(Tok::Eof);
        Ok(())
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
                    return Err(format!(
                        "SyntaxError: unterminated regular expression (line {})",
                        self.line
                    ))
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
        if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            return self.scan_name();
        }
        // Private class member (`#name`): scanned as an identifier keeping the `#`.
        if c == '#'
            && self
                .peek_at(1)
                .map(|d| d.is_ascii_alphabetic() || d == '_' || d == '$')
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
                None => {
                    return Err(format!(
                        "SyntaxError: unterminated string (line {})",
                        self.line
                    ))
                }
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    self.bump();
                    if let Some(e) = self.bump() {
                        push_escape(&mut raw, e, self);
                    }
                }
                Some('\n') => {
                    return Err(format!(
                        "SyntaxError: unterminated string literal (line {})",
                        self.line
                    ))
                }
                Some(c) => {
                    raw.push(c);
                    self.bump();
                }
            }
        }
        self.push(Tok::Str(raw));
        Ok(())
    }

    /// Scan a `` `...${expr}...` `` template. Cooked quasis are decoded; each
    /// `${...}` field's raw source (with balanced braces) is captured for the
    /// parser to re-parse.
    fn scan_template(&mut self) -> Result<(), String> {
        self.bump(); // opening backtick
        let mut quasis = Vec::new();
        let mut raws = Vec::new();
        let mut exprs = Vec::new();
        let mut cur = String::new();
        let mut cur_raw = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(format!(
                        "SyntaxError: unterminated template (line {})",
                        self.line
                    ))
                }
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
                        push_escape(&mut cur, e, self);
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
                    // Capture raw source until the matching `}` (brace-balanced,
                    // skipping strings).
                    let mut depth = 1;
                    let mut src = String::new();
                    loop {
                        match self.peek() {
                            None => {
                                return Err(format!(
                                    "SyntaxError: unterminated template expression (line {})",
                                    self.line
                                ))
                            }
                            Some('{') => {
                                depth += 1;
                                src.push('{');
                                self.bump();
                            }
                            Some('}') => {
                                depth -= 1;
                                self.bump();
                                if depth == 0 {
                                    break;
                                }
                                src.push('}');
                            }
                            Some(q) if q == '"' || q == '\'' || q == '`' => {
                                src.push(q);
                                self.bump();
                                while let Some(cc) = self.peek() {
                                    src.push(cc);
                                    self.bump();
                                    if cc == '\\' {
                                        if let Some(n) = self.peek() {
                                            src.push(n);
                                            self.bump();
                                        }
                                    } else if cc == q {
                                        break;
                                    }
                                }
                            }
                            Some(cc) => {
                                src.push(cc);
                                self.bump();
                            }
                        }
                    }
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
        });
        Ok(())
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
                    while let Some(c) = self.peek() {
                        if c == '_' {
                            self.pos += 1;
                        } else if c.is_digit(radix) {
                            digits.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    // `0x..n` / `0o..n` / `0b..n` BigInt literal: the digits carry
                    // arbitrary precision, so parse them as a bignum (radix-aware)
                    // rather than through `i64`.
                    if self.peek() == Some('n') {
                        self.pos += 1;
                        let big = num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)
                            .ok_or_else(|| {
                                format!("SyntaxError: bad bigint (line {})", self.line)
                            })?;
                        self.push(Tok::BigInt(big.to_string()));
                        return Ok(());
                    }
                    let n = i64::from_str_radix(&digits, radix)
                        .map_err(|_| format!("SyntaxError: bad number (line {})", self.line))?;
                    self.push(Tok::Num(n as f64));
                    return Ok(());
                }
            }
        }
        let mut s = String::new();
        while let Some(c) = self.peek() {
            match c {
                '0'..='9' => {
                    s.push(c);
                    self.pos += 1;
                }
                '_' => {
                    self.pos += 1;
                }
                '.' => {
                    s.push(c);
                    self.pos += 1;
                }
                'e' | 'E' => {
                    s.push('e');
                    self.pos += 1;
                    if matches!(self.peek(), Some('+') | Some('-')) {
                        s.push(self.peek().unwrap());
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
        // Decimal `BigInt` literal (`123n`): only integer digit runs may carry the
        // `n` suffix (a `.`/`e` makes it an ordinary number, and `1.5n` is a
        // SyntaxError in JS — we leave the `n` as a stray identifier so it fails).
        if self.peek() == Some('n') && !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
            self.pos += 1;
            self.push(Tok::BigInt(s));
            return Ok(());
        }
        let v: f64 = s
            .parse()
            .map_err(|_| format!("SyntaxError: bad number '{s}' (line {})", self.line))?;
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
            Err(format!(
                "SyntaxError: unexpected character {c:?} (line {})",
                self.line
            ))
        }
    }
}

/// Append one escape sequence's decoded character(s) to `out`. `\xNN` and
/// `\uNNNN` / `\u{...}` are decoded; unknown escapes keep the literal char.
fn push_escape(out: &mut String, e: char, lx: &mut Lexer) {
    match e {
        'n' => out.push('\n'),
        't' => out.push('\t'),
        'r' => out.push('\r'),
        'b' => out.push('\u{08}'),
        'f' => out.push('\u{0C}'),
        'v' => out.push('\u{0B}'),
        '0' => out.push('\0'),
        '\\' => out.push('\\'),
        '\'' => out.push('\''),
        '"' => out.push('"'),
        '`' => out.push('`'),
        '\n' => {} // line continuation
        'x' => {
            let mut h = String::new();
            for _ in 0..2 {
                if let Some(c) = lx.peek() {
                    if c.is_ascii_hexdigit() {
                        h.push(c);
                        lx.bump();
                    }
                }
            }
            if let Ok(n) = u32::from_str_radix(&h, 16) {
                if let Some(ch) = char::from_u32(n) {
                    out.push(ch);
                }
            }
        }
        'u' => {
            if lx.peek() == Some('{') {
                lx.bump();
                let mut h = String::new();
                while let Some(c) = lx.peek() {
                    if c == '}' {
                        lx.bump();
                        break;
                    }
                    h.push(c);
                    lx.bump();
                }
                if let Ok(n) = u32::from_str_radix(&h, 16) {
                    if let Some(ch) = char::from_u32(n) {
                        out.push(ch);
                    }
                }
            } else {
                let mut h = String::new();
                for _ in 0..4 {
                    if let Some(c) = lx.peek() {
                        if c.is_ascii_hexdigit() {
                            h.push(c);
                            lx.bump();
                        }
                    }
                }
                if let Ok(n) = u32::from_str_radix(&h, 16) {
                    if let Some(ch) = char::from_u32(n) {
                        out.push(ch);
                    }
                }
            }
        }
        other => out.push(other),
    }
}
