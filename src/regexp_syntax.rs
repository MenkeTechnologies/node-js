//! Early-error validation of a JavaScript regular-expression pattern.
//!
//! [`crate::regexp`] translates a pattern into fancy-regex syntax and lets that
//! engine reject what it cannot compile — which accepts patterns JavaScript
//! refuses (`/{/u`, `/\-/u`, a duplicate group name) and words the ones it does
//! refuse in its own vocabulary. This module is the pattern GRAMMAR of
//! ECMA-262 22.2.1 (with the Annex B.1.2 relaxations outside unicode mode),
//! walked once before translation, reporting V8's wording
//! (`src/regexp/regexp-error.h`) for the first early error.
//!
//! It only ever REJECTS: a pattern it accepts still goes through `translate`
//! and the engine, so a gap here costs a worse message, never a wrong match.
//! `v` (unicodeSets) class bodies nest and use set operators that this walk
//! does not model; their brackets are balanced and their contents left to the
//! engine.

/// The first early error in `pat`, with V8's reason text.
pub fn validate(pat: &str, unicode: bool, sets: bool) -> Result<(), String> {
    let chars: Vec<char> = pat.chars().collect();
    let (group_total, names) = group_census(&chars);
    let mut p = Parser {
        c: &chars,
        i: 0,
        unicode,
        sets,
        group_total,
        has_named: !names.is_empty(),
        all_names: names,
        path_names: Vec::new(),
    };
    p.disjunction()?;
    if p.i < p.c.len() {
        // `disjunction` stops at an unmatched `)`.
        return Err("Unmatched ')'".to_string());
    }
    Ok(())
}

/// How many capturing groups the pattern has and the names they carry. A decimal
/// escape is a backreference only when it does not exceed the TOTAL count, and
/// `\k` is only a named reference when a name exists anywhere in the pattern —
/// both are decided before the walk reaches the group, so they are counted first.
fn group_census(c: &[char]) -> (usize, Vec<String>) {
    let mut count = 0;
    let mut names = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '(' if !in_class => {
                if c.get(i + 1) != Some(&'?') {
                    count += 1;
                } else if c.get(i + 2) == Some(&'<') && !matches!(c.get(i + 3), Some('=' | '!')) {
                    count += 1;
                    names.push(c[i + 3..].iter().take_while(|ch| **ch != '>').collect());
                }
            }
            _ => {}
        }
        i += 1;
    }
    (count, names)
}

/// Whether a quantifier may follow the atom just parsed.
#[derive(Clone, Copy)]
enum Quant {
    Yes,
    /// `^`, `$`, `\b`, `\B`: nothing to repeat.
    Assertion,
    /// A lookbehind (or a lookahead under `u`): a group that cannot repeat.
    Refused,
}

/// What one ClassAtom contributes to a range: a single code point, or a set
/// (`\d`, `\p{…}`) which cannot be a range endpoint in unicode mode.
enum ClassAtom {
    Char(u32),
    Set,
}

struct Parser<'a> {
    c: &'a [char],
    i: usize,
    unicode: bool,
    sets: bool,
    group_total: usize,
    has_named: bool,
    all_names: Vec<String>,
    /// Group names declared on the path from the pattern's start to here. A
    /// name may repeat only across alternatives that exclude one another.
    path_names: Vec<String>,
}

type Res<T> = Result<T, String>;

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.c.get(self.i + n).copied()
    }

    fn disjunction(&mut self) -> Res<()> {
        let base = self.path_names.len();
        let mut declared_in_any: Vec<String> = Vec::new();
        loop {
            self.path_names.truncate(base);
            self.alternative()?;
            declared_in_any.extend(self.path_names.drain(base..));
            if self.peek() == Some('|') {
                self.i += 1;
            } else {
                break;
            }
        }
        // After the disjunction every alternative's names are on the path.
        self.path_names.extend(declared_in_any);
        Ok(())
    }

    fn alternative(&mut self) -> Res<()> {
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            self.term()?;
        }
        Ok(())
    }

    fn term(&mut self) -> Res<()> {
        let c = self.peek().expect("term called at end of pattern");
        // What may follow the atom. `Assertion` (`^ $ \b \B`) takes no quantifier
        // at all (`Nothing to repeat`); a lookbehind, or a lookahead under `u`,
        // is a group that merely refuses one (`Invalid quantifier`).
        let quantifiable = match c {
            '^' | '$' => {
                self.i += 1;
                Quant::Assertion
            }
            '(' => self.group()?,
            '\\' => self.atom_escape()?,
            '[' => {
                self.class()?;
                Quant::Yes
            }
            '*' | '+' | '?' => return Err("Nothing to repeat".to_string()),
            '{' => {
                if self.braces_quantifier()?.is_some() {
                    return Err("Nothing to repeat".to_string());
                }
                if self.unicode {
                    return Err("Lone quantifier brackets".to_string());
                }
                self.i += 1;
                Quant::Yes
            }
            '}' | ']' if self.unicode => {
                return Err("Lone quantifier brackets".to_string());
            }
            _ => {
                self.i += 1;
                Quant::Yes
            }
        };
        self.quantifier(quantifiable)
    }

    /// An optional quantifier after an atom. `quantifiable` is false for
    /// assertions, which refuse one (`/^*/`, `/\b+/`, a lookbehind).
    fn quantifier(&mut self, quantifiable: Quant) -> Res<()> {
        let quantified = match self.peek() {
            Some('*' | '+' | '?') => {
                self.i += 1;
                true
            }
            Some('{') => match self.braces_quantifier()? {
                Some(()) => true,
                None if self.unicode => return Err("Incomplete quantifier".to_string()),
                // Outside unicode mode a malformed `{` is just a literal.
                None => false,
            },
            _ => false,
        };
        if quantified {
            match quantifiable {
                Quant::Yes => {}
                Quant::Assertion => return Err("Nothing to repeat".to_string()),
                Quant::Refused => return Err("Invalid quantifier".to_string()),
            }
            if self.peek() == Some('?') {
                self.i += 1; // lazy
            }
        }
        Ok(())
    }

    /// A `{n}`, `{n,}` or `{n,m}` at the cursor. `Ok(Some)` consumes it,
    /// `Ok(None)` leaves the cursor alone (not a quantifier), and `n > m` is the
    /// error V8 words as "numbers out of order".
    fn braces_quantifier(&mut self) -> Res<Option<()>> {
        let mut j = self.i + 1;
        let digits = |j: &mut usize| {
            let start = *j;
            while self.c.get(*j).is_some_and(char::is_ascii_digit) {
                *j += 1;
            }
            self.c[start..*j].iter().collect::<String>()
        };
        let lo = digits(&mut j);
        if lo.is_empty() {
            return Ok(None);
        }
        let mut hi = None;
        if self.c.get(j) == Some(&',') {
            j += 1;
            let h = digits(&mut j);
            hi = Some(h);
        }
        if self.c.get(j) != Some(&'}') {
            return Ok(None);
        }
        if let Some(h) = hi.filter(|h| !h.is_empty()) {
            // Compared as decimal strings of arbitrary length, since either
            // bound may exceed every machine integer.
            let norm = |s: &str| s.trim_start_matches('0').to_string();
            let (a, b) = (norm(&lo), norm(&h));
            if a.len() > b.len() || (a.len() == b.len() && a > b) {
                return Err("numbers out of order in {} quantifier".to_string());
            }
        }
        self.i = j + 1;
        Ok(Some(()))
    }

    /// A parenthesised group. Returns whether a quantifier may follow it.
    fn group(&mut self) -> Res<Quant> {
        self.i += 1; // (
        let mut quantifiable = Quant::Yes;
        if self.peek() == Some('?') {
            match (self.peek_at(1), self.peek_at(2)) {
                (Some(':'), _) => self.i += 2,
                (Some('=' | '!'), _) => {
                    self.i += 2;
                    // Annex B.1.2: a lookahead is quantifiable outside `u`.
                    if self.unicode {
                        quantifiable = Quant::Refused;
                    }
                }
                (Some('<'), Some('=' | '!')) => {
                    self.i += 3;
                    quantifiable = Quant::Refused;
                }
                (Some('<'), _) => {
                    self.i += 2;
                    let name = self.group_name()?;
                    if self.path_names.contains(&name) {
                        return Err("Duplicate capture group name".to_string());
                    }
                    self.path_names.push(name);
                }
                // `(?ims-ims:` modifiers: judged by `regexp::check_group`.
                _ => {
                    self.i += 1;
                    while self
                        .peek()
                        .is_some_and(|c| matches!(c, 'i' | 'm' | 's' | '-'))
                    {
                        self.i += 1;
                    }
                    if self.peek() != Some(':') {
                        return Err("Invalid group".to_string());
                    }
                    self.i += 1;
                }
            }
        }
        self.disjunction()?;
        if self.peek() != Some(')') {
            return Err("Unterminated group".to_string());
        }
        self.i += 1;
        Ok(quantifiable)
    }

    /// The `name>` of a `(?<name>` / `\k<name>`, cursor just past the `<`.
    fn group_name(&mut self) -> Res<String> {
        const BAD: &str = "Invalid capture group name";
        let mut name = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(BAD.to_string());
            };
            self.i += 1;
            let ch = match c {
                '>' => break,
                '\\' if self.peek() == Some('u') => {
                    self.i += 1;
                    self.unicode_escape_value(true)
                        .ok_or_else(|| BAD.to_string())?
                }
                c => c as u32,
            };
            let ch = char::from_u32(ch).ok_or_else(|| BAD.to_string())?;
            let ok = if name.is_empty() {
                ch == '$' || ch == '_' || ch.is_alphabetic()
            } else {
                ch == '$'
                    || ch == '_'
                    || ch.is_alphanumeric()
                    || ch == '\u{200C}'
                    || ch == '\u{200D}'
            };
            if !ok {
                return Err(BAD.to_string());
            }
            name.push(ch);
        }
        if name.is_empty() {
            return Err(BAD.to_string());
        }
        Ok(name)
    }

    /// After `\u`: the code point of `XXXX` (with a following `\uXXXX` low
    /// surrogate folded in) or, when `braces`, of `{X…}`. Consumes on success.
    fn unicode_escape_value(&mut self, braces: bool) -> Option<u32> {
        let hex = |p: &Self, from: usize, n: usize| -> Option<u32> {
            let mut v = 0;
            for k in 0..n {
                v = v * 16 + p.c.get(from + k)?.to_digit(16)?;
            }
            Some(v)
        };
        if braces && self.peek() == Some('{') {
            let mut j = self.i + 1;
            let mut v: u32 = 0;
            let mut any = false;
            while let Some(d) = self.c.get(j).and_then(|c| c.to_digit(16)) {
                v = v.saturating_mul(16).saturating_add(d);
                any = true;
                j += 1;
            }
            if !any || self.c.get(j) != Some(&'}') || v > 0x10FFFF {
                return None;
            }
            self.i = j + 1;
            return Some(v);
        }
        let hi = hex(self, self.i, 4)?;
        self.i += 4;
        if (0xD800..=0xDBFF).contains(&hi)
            && self.peek() == Some('\\')
            && self.peek_at(1) == Some('u')
        {
            if let Some(lo) = hex(self, self.i + 2, 4).filter(|l| (0xDC00..=0xDFFF).contains(l)) {
                self.i += 6;
                return Some(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00));
            }
        }
        Some(hi)
    }

    /// `\` outside a class. Returns whether a quantifier may follow.
    fn atom_escape(&mut self) -> Res<Quant> {
        self.i += 1; // backslash
        let Some(c) = self.peek() else {
            return Err("\\ at end of pattern".to_string());
        };
        match c {
            'b' | 'B' => {
                self.i += 1;
                Ok(Quant::Assertion)
            }
            '1'..='9' => {
                let start = self.i;
                while self.peek().is_some_and(|d| d.is_ascii_digit()) {
                    self.i += 1;
                }
                let n: usize = self.c[start..self.i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(usize::MAX);
                if n > self.group_total {
                    if self.unicode {
                        return Err("Invalid escape".to_string());
                    }
                    // Annex B: a legacy octal escape, or the digit itself.
                    self.i = start + 1;
                }
                Ok(Quant::Yes)
            }
            'k' if self.unicode || self.has_named => {
                self.i += 1;
                if self.peek() != Some('<') {
                    return Err("Invalid named reference".to_string());
                }
                self.i += 1;
                let name = self
                    .group_name()
                    .map_err(|_| "Invalid named reference".to_string())?;
                if !self.all_names.contains(&name) {
                    return Err("Invalid named capture referenced".to_string());
                }
                Ok(Quant::Yes)
            }
            _ => {
                self.character_escape(false)?;
                Ok(Quant::Yes)
            }
        }
    }

    /// The escape after a `\` that stands for one character or a class set,
    /// cursor on its first character. Shared by atoms and class atoms.
    fn character_escape(&mut self, in_class: bool) -> Res<ClassAtom> {
        let c = self.peek().expect("caller checked for end of pattern");
        self.i += 1;
        let ch = |v: u32| Ok(ClassAtom::Char(v));
        match c {
            'd' | 'D' | 's' | 'S' | 'w' | 'W' => Ok(ClassAtom::Set),
            'f' => ch(0x0C),
            'n' => ch(0x0A),
            'r' => ch(0x0D),
            't' => ch(0x09),
            'v' => ch(0x0B),
            'b' if in_class => ch(0x08),
            'c' => match self.peek() {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.i += 1;
                    ch(l as u32 % 32)
                }
                Some(l) if in_class && !self.unicode && (l.is_ascii_digit() || l == '_') => {
                    self.i += 1;
                    ch(l as u32 % 32)
                }
                _ if self.unicode => Err("Invalid Unicode escape".to_string()),
                // Annex B: the backslash stands for itself; `c` is read next.
                _ => {
                    self.i -= 1;
                    ch('\\' as u32)
                }
            },
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => ch(0),
            '0'..='7' if !self.unicode => {
                // Legacy octal escape.
                let mut v = c.to_digit(8).unwrap_or(0);
                let max = if c <= '3' { 3 } else { 2 };
                for _ in 1..max {
                    match self.peek().and_then(|d| d.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.i += 1;
                        }
                        None => break,
                    }
                }
                ch(v)
            }
            '0'..='9' if self.unicode => Err(if in_class || c == '0' {
                "Invalid decimal escape".to_string()
            } else {
                "Invalid escape".to_string()
            }),
            '8' | '9' => ch(c as u32),
            'x' => {
                let h = |k: usize| self.c.get(self.i + k).and_then(|d| d.to_digit(16));
                match (h(0), h(1)) {
                    (Some(a), Some(b)) => {
                        self.i += 2;
                        ch(a * 16 + b)
                    }
                    _ if self.unicode => Err("Invalid escape".to_string()),
                    _ => ch('x' as u32),
                }
            }
            'u' => match self.unicode_escape_value(self.unicode) {
                Some(v) => ch(v),
                None if self.unicode => Err("Invalid Unicode escape".to_string()),
                None => ch('u' as u32),
            },
            'p' | 'P' if self.unicode => {
                if self.peek() != Some('{') {
                    return Err(property_error(in_class));
                }
                let close = self.c[self.i..].iter().position(|&d| d == '}');
                let Some(close) = close else {
                    return Err(property_error(in_class));
                };
                let name: String = self.c[self.i + 1..self.i + close].iter().collect();
                if !property_is_known(&name) {
                    return Err(property_error(in_class));
                }
                self.i += close + 1;
                Ok(ClassAtom::Set)
            }
            // SyntaxCharacter and `/` are always identity escapes; `-` only
            // inside a class under `u`.
            '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
            | '/' => ch(c as u32),
            '-' if in_class || !self.unicode => ch('-' as u32),
            // ClassSetReservedPunctuator: escapable inside a `v` class.
            '&' | '!' | '#' | '%' | ',' | ':' | ';' | '<' | '=' | '>' | '@' | '`' | '~'
                if in_class && self.sets =>
            {
                ch(c as u32)
            }
            _ if self.unicode => Err("Invalid escape".to_string()),
            _ => ch(c as u32),
        }
    }

    /// A `[...]` class, cursor on the `[`.
    fn class(&mut self) -> Res<()> {
        const UNTERMINATED: &str = "Unterminated character class";
        self.i += 1; // [
        if self.peek() == Some('^') {
            self.i += 1;
        }
        loop {
            match self.peek() {
                None => return Err(UNTERMINATED.to_string()),
                Some(']') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => {}
            }
            if self.sets && self.set_syntax()? {
                continue;
            }
            let lo = self.class_atom()?;
            if self.peek() == Some('-')
                && self.peek_at(1).is_some_and(|c| c != ']')
                && !(self.sets && self.peek_at(1) == Some('-'))
            {
                self.i += 1;
                let hi = self.class_atom()?;
                match (lo, hi) {
                    (ClassAtom::Char(a), ClassAtom::Char(b)) if a > b => {
                        return Err("Range out of order in character class".to_string());
                    }
                    (ClassAtom::Set, _) | (_, ClassAtom::Set) if self.unicode => {
                        return Err("Invalid character class".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    fn class_atom(&mut self) -> Res<ClassAtom> {
        let c = self
            .peek()
            .ok_or_else(|| "Unterminated character class".to_string())?;
        if c != '\\' {
            self.i += 1;
            return Ok(ClassAtom::Char(c as u32));
        }
        self.i += 1;
        match self.peek() {
            None => Err("\\ at end of pattern".to_string()),
            // `\k`, `\B` and the decimal forms inside a class are judged by
            // `regexp::translate`, which owns their wording.
            Some('B') if self.unicode => Err("Invalid escape".to_string()),
            Some('k' | 'B') => {
                let k = self.peek().unwrap_or('k');
                self.i += 1;
                Ok(ClassAtom::Char(k as u32))
            }
            Some(_) => self.character_escape(true),
        }
    }

    /// The `v`-mode (unicodeSets) members of a class that are not a plain
    /// ClassAtom: a nested class, the `&&` and `--` operators, a `\q{…}` string
    /// disjunction, and the ClassSetSyntaxCharacters that may not appear bare.
    /// Returns true when it consumed something.
    fn set_syntax(&mut self) -> Res<bool> {
        match (self.peek(), self.peek_at(1)) {
            (Some('['), _) => {
                self.class()?;
                Ok(true)
            }
            (Some('&'), Some('&')) | (Some('-'), Some('-')) => {
                self.i += 2;
                Ok(true)
            }
            (Some('\\'), Some('q')) if self.peek_at(2) == Some('{') => {
                while self.peek().is_some_and(|c| c != '}') {
                    self.i += 1;
                }
                self.i += 1;
                Ok(true)
            }
            (Some('(' | ')' | '{' | '}' | '/' | '|'), _) => {
                Err("Invalid character in character class".to_string())
            }
            _ => Ok(false),
        }
    }
}

fn property_error(in_class: bool) -> String {
    if in_class {
        "Invalid property name in character class".to_string()
    } else {
        "Invalid property name".to_string()
    }
}

/// Whether `name` (the text inside `\p{…}`) names a property the engine knows.
/// The set JavaScript accepts is the Unicode-property subset ECMA-262 lists
/// (Table 68/69), which the regex layer's property tables cover; asking it to
/// compile `\p{name}` is the lookup.
fn property_is_known(name: &str) -> bool {
    !name.is_empty() && regex::Regex::new(&format!("\\p{{{name}}}")).is_ok()
}
