//! The UTF-8 ⇄ UTF-16 boundary for JS string indices.
//!
//! A JS `String` is a sequence of UTF-16 code units, and *every* index-bearing
//! `String.prototype` operation counts in those units: `length`, `charAt`,
//! `charCodeAt`, `codePointAt`, `at`, `indexOf`/`lastIndexOf`, `slice`,
//! `substring`, `substr`, `split`, `padStart`/`padEnd`, `s[i]`, and a RegExp's
//! `.index`/`lastIndex`.
//!
//! node-js stores a JS string as a Rust `String` (UTF-8): `fusevm::Value::Str`
//! and the host heap's `JsObj::Str` are both `String`, and `fusevm` is a pinned
//! external dependency, so the storage type is not ours to change. Every
//! JS-visible index therefore has to be translated, and this module is the one
//! place that translation happens. Indices are code-unit counts (`U16Index`);
//! Rust's own string offsets are byte counts; the two are only equal on ASCII,
//! and the newtype exists so a function that has both in scope cannot silently
//! pass one where the other belongs.
//!
//! # The lone-surrogate boundary
//!
//! Rust `String` cannot hold an unpaired surrogate — `char` excludes
//! `U+D800..=U+DFFF` — so an operation that *cuts a surrogate pair in half*
//! cannot reproduce node's result exactly. `"𝒳".charAt(0)` is the lone
//! surrogate `\ud835` in node; here it is `U+FFFD`. This is deliberately the
//! narrowest possible gap:
//!
//! * Index *arithmetic* is exact — a cut at a surrogate boundary still happens
//!   at the right place, still yields a 1-unit string, and every surrounding
//!   index still lines up. `"𝒳".length` is `2` and `"𝒳".charCodeAt(0)` is
//!   `55349`, read from the intact original.
//! * Printing is byte-identical: node itself writes `ef bf bd` (U+FFFD) when a
//!   lone surrogate reaches stdout, verified with
//!   `node -e 'process.stdout.write("𝒳".charAt(0))' | xxd`.
//! * Only *re-inspecting an extracted half* differs — `"𝒳".charAt(0).charCodeAt(0)`
//!   (65533 here, 55349 in node), `JSON.stringify("𝒳".charAt(0))`, and
//!   re-joining two halves back into the original astral character.
//!
//! Closing that last gap means replacing `String` with a WTF-8 buffer
//! throughout `fusevm` and all 47 stdlib modules, which the pinned dependency
//! forbids.

/// An index into a JS string, counted in UTF-16 code units.
///
/// Distinct from a Rust byte offset on purpose: the regex path holds both at
/// once (a match's byte offsets, a `lastIndex` in code units) and mixing them
/// is the exact bug this module exists to prevent.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct U16Index(usize);

impl U16Index {
    pub const ZERO: U16Index = U16Index(0);

    pub fn new(i: usize) -> Self {
        U16Index(i)
    }

    pub fn get(self) -> usize {
        self.0
    }
}

/// The length of `s` in UTF-16 code units — the value of `s.length` in JS.
pub fn len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// A JS string decoded into its UTF-16 code units, so that index arithmetic can
/// be done directly on the units JS counts.
pub struct Units(Vec<u16>);

impl Units {
    pub fn of(s: &str) -> Self {
        Units(s.encode_utf16().collect())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[u16] {
        &self.0
    }

    /// The code unit at `i` — the value `charCodeAt(i)` reports.
    pub fn unit(&self, i: usize) -> Option<u16> {
        self.0.get(i).copied()
    }

    /// The code *point* starting at `i`: a full astral scalar when `i` is the
    /// leading half of a surrogate pair, otherwise the bare unit. This is
    /// `codePointAt`, which — unlike `charCodeAt` — looks ahead one unit.
    pub fn code_point(&self, i: usize) -> Option<u32> {
        let hi = self.unit(i)? as u32;
        if (0xD800..0xDC00).contains(&hi) {
            if let Some(lo) = self.unit(i + 1).map(u32::from) {
                if (0xDC00..0xE000).contains(&lo) {
                    return Some(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00));
                }
            }
        }
        Some(hi)
    }

    /// The substring `[lo, hi)` in code units. An index that splits a surrogate
    /// pair yields `U+FFFD` for the orphaned half — see the module docs.
    pub fn slice(&self, lo: usize, hi: usize) -> String {
        let lo = lo.min(self.0.len());
        let hi = hi.clamp(lo, self.0.len());
        to_string_lossy(&self.0[lo..hi])
    }

    /// The single code unit at `i` as a string — `charAt(i)` / `s[i]`.
    pub fn unit_str(&self, i: usize) -> Option<String> {
        self.unit(i).map(|u| to_string_lossy(&[u]))
    }
}

/// Decode UTF-16 code units back to a Rust `String`, mapping any unpaired
/// surrogate to `U+FFFD` — the same replacement node performs when a lone
/// surrogate is written to stdout.
pub fn to_string_lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `ToUint16(n)` — the modulo-2^16 wrap `String.fromCharCode` applies to each
/// argument, so `fromCharCode(0x1D4B3)` produces U+D4B3 and not U+1D4B3.
pub fn to_uint16(n: f64) -> u16 {
    if !n.is_finite() {
        return 0;
    }
    (n.trunc().rem_euclid(65536.0)) as u16
}

/// The UTF-16 index corresponding to a UTF-8 *byte* offset into `s`.
pub fn index_of_byte(s: &str, byte: usize) -> U16Index {
    let byte = byte.min(s.len());
    // Round a non-boundary byte down so the slice below is always valid.
    let mut b = byte;
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    U16Index(len(&s[..b]))
}

/// The UTF-8 byte offset corresponding to a UTF-16 index into `s`. An index
/// that falls *inside* a surrogate pair rounds down to the start of that code
/// point, so the result is always a valid `str` boundary.
pub fn byte_of_index(s: &str, idx: U16Index) -> usize {
    let target = idx.get();
    let mut units = 0usize;
    for (b, c) in s.char_indices() {
        if units + c.len_utf16() > target {
            return b;
        }
        units += c.len_utf16();
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured node values for `"𝒳"` (U+1D4B3, one code point, two units)
    /// and `"ab𝒳cd"`, from `node v26.7.0`.
    #[test]
    fn astral_lengths_and_units() {
        assert_eq!(len("𝒳"), 2);
        assert_eq!(len("ab𝒳cd"), 6);
        assert_eq!(len("😀🎉"), 4);
        assert_eq!(len("abc"), 3);

        let u = Units::of("𝒳");
        assert_eq!(u.len(), 2);
        assert_eq!(u.unit(0), Some(55349));
        assert_eq!(u.unit(1), Some(56499));
        // codePointAt looks ahead; charCodeAt does not.
        assert_eq!(u.code_point(0), Some(119987));
        assert_eq!(u.code_point(1), Some(56499));
        assert_eq!(u.unit(2), None);
    }

    #[test]
    fn slicing_a_pair_in_half_keeps_the_unit_count() {
        let u = Units::of("𝒳");
        // node yields a lone surrogate here; we yield U+FFFD, which is still
        // exactly one code unit, so every downstream index still lines up.
        assert_eq!(len(&u.slice(0, 1)), 1);
        assert_eq!(len(&u.slice(1, 2)), 1);
        assert_eq!(u.slice(0, 2), "𝒳");
        assert_eq!(u.slice(3, 9), "");
    }

    /// A code-unit index and a byte offset must round-trip through each other.
    #[test]
    fn byte_and_index_round_trip() {
        let s = "ab𝒳cd";
        // 'c' is at byte 6 and at UTF-16 index 4 (node: "ab𝒳cd".indexOf("c") === 4).
        assert_eq!(index_of_byte(s, 6), U16Index::new(4));
        assert_eq!(byte_of_index(s, U16Index::new(4)), 6);
        assert_eq!(index_of_byte(s, 0), U16Index::ZERO);
        assert_eq!(byte_of_index(s, U16Index::new(0)), 0);
        assert_eq!(byte_of_index(s, U16Index::new(99)), s.len());
        // Index 3 splits the surrogate pair: round down to the pair's start.
        assert_eq!(byte_of_index(s, U16Index::new(3)), 2);
    }

    #[test]
    fn index_of_byte_tolerates_a_non_boundary_offset() {
        let s = "ab𝒳cd";
        // Bytes 3..5 are continuation bytes of the astral char; all round down
        // to the char start, which is UTF-16 index 2.
        for b in 2..=5 {
            assert_eq!(index_of_byte(s, b), U16Index::new(2), "byte {b}");
        }
    }
}
