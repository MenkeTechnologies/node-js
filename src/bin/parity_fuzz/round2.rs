//! Round-two generator modes: the areas the first twelve rounds' modes left
//! unprobed — Date parsing and formatting, `JSON.stringify` edge cases,
//! `Number#toString(radix)`/`toFixed`/`toPrecision`/`toExponential`, the
//! `String.prototype` methods over non-ASCII text, `Array` methods over holes
//! and species, Proxy/Reflect invariants, microtask/timer ordering, class
//! fields/private names/static blocks, destructuring and iterator closing,
//! completion values, accessors on prototype chains, and the `Symbol.*`
//! protocols.
//!
//! Every program prints through `console.log` and either `P` (value or
//! `Name: message`) or a log array joined once, so one divergence names one
//! observable. Nothing here reads a clock, an address or an unstable ordering.

use super::{pick, Rng, PROBE};

/// Fill `$NAME` placeholders in a JS template. The generators below are mostly
/// JS text with a few holes, and `format!` would make every brace a `{{`.
fn tpl(text: &str, subs: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (k, v) in subs {
        out = out.replace(k, v);
    }
    out
}

fn probe() -> String {
    PROBE.to_string()
}

// ── Date ─────────────────────────────────────────────────────────────────────

/// `Date.parse` / `new Date(string)` over the ISO profile (21.4.1.32), the two
/// formats `toString`/`toUTCString` emit, and V8's legacy fallback, plus the
/// readable string forms of the resulting time value.
pub fn gen_dateparse(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const STRINGS: &[&str] = &[
        "2020-01-01",
        "2020-01-01T00:00:00Z",
        "2020-01-01T00:00:00.123Z",
        "2020-01-01T00:00:00.123456Z",
        "2020-01-01T00:00",
        "2020-01-01T10:20",
        "2020-01",
        "2020",
        "+002020-01-01T00:00:00Z",
        "-000001-01-01T00:00:00Z",
        "-000000-01-01T00:00:00Z",
        "+275760-09-13T00:00:00.000Z",
        "+275760-09-13T00:00:00.001Z",
        "-271821-04-20T00:00:00.000Z",
        "-271821-04-19T23:59:59.999Z",
        "2020-13-01",
        "2020-00-10",
        "2020-02-30",
        "2020-02-29T24:00:00Z",
        "2020-02-29T24:00:01Z",
        "2020-01-01T25:00:00Z",
        "2020-01-01T00:60:00Z",
        "2020-01-01T00:00:60Z",
        "2020-01-01T00:00:00+05:30",
        "2020-01-01T00:00:00-0530",
        "2020-01-01T00:00:00+0530",
        "2020-01-01T00:00:00-05",
        "2020-01-01 00:00:00",
        "2020-01-01 00:00:00Z",
        "2020-01-01T00:00:00.1Z",
        "2020-01-01T00:00:00.12345678901Z",
        "2020-1-1",
        "2020-01-1",
        "Jan 1 2020",
        "Jan 1, 2020",
        "January 1, 2020 10:00:00",
        "1 January 2020",
        "Wed, 01 Jan 2020 00:00:00 GMT",
        "Wed, 01 Jan 2020 00:00:00 +0100",
        "Wed Jan 01 2020 00:00:00 GMT+0000 (Coordinated Universal Time)",
        "Wed Jan 01 2020 00:00:00 GMT-0500",
        "Wed Jan 01 2020",
        "2020/01/01",
        "2020/1/1 10:00",
        "01/02/2020",
        "1/2/99",
        "12/31/1999 23:59:59",
        "12/31/1999 11:59 PM",
        "12/31/1999 12:00 AM",
        "Jan 32 2020",
        "Feb 29 2021",
        "Feb 29 2020",
        "Thu, 01 Jan 1970 00:00:00 GMT",
        "Thu, 01 Jan 1970 00:00:00 UTC",
        "Thu, 01 Jan 1970 00:00:00 Z",
        "Thu, 01 Jan 1970 00:00:00 EST",
        "Thu, 01 Jan 1970 00:00:00 PDT",
        "2020-01-01T00:00:00 GMT",
        "2020-06-15T12:30:45.678Z",
        "Sat, 31 Dec 2022 23:59:60 GMT",
        "Sat, 31 Dec 2022 23:59:59 GMT",
        "T00:00",
        "12:00",
        "",
        " ",
        "  2020-01-01  ",
        "2020-01-01T",
        "2020-01-01T00:00:00ZZ",
        "99",
        "100",
        "1",
        "12",
        "13",
        "Mon",
        "May",
        "May 2020",
        "May 15",
        "2020 May 15",
        "15 May 2020 10:30 GMT+0200",
        "Thu Jan 01 1970 00:00:00 GMT+0000 (UTC)",
        "1970-01-01T00:00:00.000+00:00",
        "1970-01-01T00:00:00.000-00:00",
        "0000-01-01T00:00:00Z",
        "0001-01-01T00:00:00Z",
        "9999-12-31T23:59:59.999Z",
        "10000-01-01T00:00:00Z",
        "2020-W01",
        "2020-001",
        "20200101",
        "20200101T000000Z",
        "Tuesday, March 3, 2020",
        "3 March 2020, 4:05:06 pm",
        "2020-03-03T04:05:06.789+01:00",
        "NaN",
        "undefined",
        "null",
        "true",
        "Infinity",
    ];
    let a = pick(r, STRINGS);
    let b = pick(r, STRINGS);
    match r.below(4) {
        0 => vec![
            probe(),
            tpl(
                "for (const s of [$A, $B]) console.log(JSON.stringify(s), P(() => Date.parse(s)), P(() => new Date(s).toISOString()));",
                &[("$A", &js_str(a)), ("$B", &js_str(b))],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const d = new Date($A);\nconsole.log(P(() => String(d)), P(() => d.toUTCString()), P(() => d.toDateString()), P(() => d.toTimeString()));\nconsole.log(P(() => d.toJSON()), P(() => d.getTime()), P(() => d.getUTCDay()), P(() => d.toLocaleDateString('en-US')), P(() => d.toLocaleTimeString('en-US')));",
                &[("$A", &js_str(a))],
            ),
        ],
        2 => {
            let t = pick(
                r,
                &[
                    "0", "-1", "86399999", "951782400000", "-62198755200000", "-62167219200001",
                    "253402300799999", "253402300800000", "1e12", "-1e12", "1583020800123",
                    "8.64e15", "-8.64e15", "-99999999999999", "1234567890123", "-2208988800000",
                ],
            );
            vec![
                probe(),
                tpl(
                    "const d = new Date($T);\nconsole.log(P(() => d.toISOString()), P(() => String(d)), P(() => d.toUTCString()), P(() => d.toLocaleString('en-US')));\nconsole.log(P(() => Date.parse(d.toString())), P(() => Date.parse(d.toUTCString())), P(() => Date.parse(d.toISOString())));",
                    &[("$T", t)],
                ),
            ]
        }
        _ => vec![
            probe(),
            tpl(
                "console.log(P(() => new Date($A).getTime()), P(() => new Date($A, $B).getTime()), P(() => new Date($A, $B, $C, $D).getTime()), P(() => Date.UTC($A, $B, $C)));",
                &[
                    ("$A", pick(r, &["2020", "99", "0", "-1", "1e5", "275760", "'2020'", "NaN", "undefined", "2020.7"])),
                    ("$B", pick(r, &["0", "11", "12", "-1", "1e3", "'1'", "undefined", "NaN", "0.9"])),
                    ("$C", pick(r, &["1", "0", "31", "32", "-1", "undefined", "29"])),
                    ("$D", pick(r, &["0", "24", "25", "-1", "12", "undefined", "1e9"])),
                ],
            ),
        ],
    }
}

/// A JS string literal for a Rust `&str` that holds no quotes or backslashes.
fn js_str(s: &str) -> String {
    format!("'{s}'")
}

// ── JSON ─────────────────────────────────────────────────────────────────────

/// `JSON.stringify` / `JSON.parse` edge cases: `toJSON`, replacer functions and
/// allow-lists, indentation clamping, the values with no JSON form, boxed
/// primitives, cycles, lone surrogates, and the parser's number/escape grammar.
pub fn gen_jsonedge(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const VALUES: &[&str] = &[
        "undefined",
        "null",
        "NaN",
        "-0",
        "Infinity",
        "1e21",
        "1e-7",
        "0.1 + 0.2",
        "'a\\u2028b'",
        "'\\ud83d\\ude00'",
        "'\\x7f\\x1f\\x00'",
        "'\"\\\\/\\b\\f\\n\\r\\t'",
        "Symbol('s')",
        "() => 1",
        "[undefined, () => 1, Symbol('x'), NaN]",
        "{ a: undefined, b: () => 1, c: Symbol('x'), d: NaN }",
        "[1, , 3]",
        "new Number(3)",
        "new String('s')",
        "new Boolean(false)",
        "Object(Symbol('b'))",
        "new Date(0)",
        "new Date(NaN)",
        "new Map([[1, 2]])",
        "new Set([1])",
        "new Uint8Array([1, 2])",
        "/re/g",
        "new Error('e')",
        "{ toJSON() { return 'tj'; } }",
        "{ toJSON() { return undefined; } }",
        "{ toJSON(k) { return 'key:' + k; } }",
        "{ x: { toJSON(k) { return 'key:' + k; } } }",
        "[{ toJSON(k) { return 'key:' + k; } }]",
        "{ get g() { return 7; }, set s(v) {} }",
        "Object.create({ inherited: 1 }, { own: { value: 2, enumerable: true }, hid: { value: 3 } })",
        "{ [Symbol('k')]: 1, a: 1 }",
        "{ 2: 'b', 1: 'a', z: 1, [-1]: 'neg' }",
        "[[]]",
        "{ a: [] , b: {} }",
        "new Proxy({ a: 1 }, {})",
        "new Proxy([1, 2], {})",
        "Object.assign(() => 1, { a: 1 })",
        "{ a: { b: { c: { d: 1 } } } }",
    ];
    const SPACES: &[&str] = &[
        "undefined",
        "0",
        "1",
        "2",
        "10",
        "11",
        "-1",
        "'ab'",
        "'12345678901'",
        "'\\t'",
        "new Number(2)",
        "new String('x')",
        "true",
        "null",
        "{}",
        "3.9",
        "NaN",
    ];
    const REPLACERS: &[&str] = &[
        "null",
        "undefined",
        "['a', 'b']",
        "['a', 'a', 'b']",
        "[1, 2]",
        "['1', 0]",
        "[new String('a'), new Number(1)]",
        "(k, v) => typeof v === 'number' ? v * 2 : v",
        "function (k, v) { return k === '' ? v : (typeof v === 'object' ? undefined : v); }",
        "(k, v) => k === 'a' ? undefined : v",
        "function (k, v) { return Array.isArray(this) ? 'arr' : v; }",
        "function (k, v) { return k === 'a' ? [1, 2] : v; }",
        "{}",
        "'a'",
    ];
    match r.below(6) {
        0 => vec![
            probe(),
            tpl(
                "const v = $V;\nconsole.log(P(() => JSON.stringify(v)), P(() => JSON.stringify([v])), P(() => JSON.stringify({ k: v })));",
                &[("$V", pick(r, VALUES))],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const v = $V;\nconsole.log(P(() => JSON.stringify(v, $R)));\nconsole.log(P(() => JSON.stringify(v, $R, $S)));",
                &[
                    ("$V", pick(r, VALUES)),
                    ("$R", pick(r, REPLACERS)),
                    ("$S", pick(r, SPACES)),
                ],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "const a = { name: 'a' }; a.self = a;\nconst b = [1]; b.push({ back: b });\nconsole.log(P(() => JSON.stringify($C)));\nconsole.log(P(() => JSON.stringify({ n: 1n })), P(() => JSON.stringify([1n], (k, v) => typeof v === 'bigint' ? String(v) : v)));",
                &[("$C", pick(r, &["a", "b", "{ x: a }", "[b]", "{ x: { y: a } }"]))],
            ),
        ],
        3 => {
            const TEXTS: &[&str] = &[
                "1", "-0", "1.5e3", "1E+2", "01", "1.", ".5", "+1", "0x10", "1e", "-", "--1",
                "1e999", "-1e999", "1e-999", "123456789012345678901234567890", "0.1e1", "-0.0",
                "\"a\"", "\"\\u0041\"", "\"\\u00\"", "\"\\x41\"", "\"\\/\"", "\"tab\\there\"",
                "\"raw\ttab\"", "'single'", "[1,]", "[,1]", "{\"a\":1,}", "{a:1}",
                "{\"a\":1,\"a\":2}", "{\"__proto__\":1}", "{\"__proto__\":{\"x\":1}}", "[]", "{}",
                "[1 2]", "{\"a\" 1}", "true", "True", "null", "nul", "undefined", "NaN", "Infinity",
                "", " ", "\n1\t", "1 2", "[1]]", "\"\\u2028\"", "\u{a0}1", "\"a\" ", "[[[[[1]]]]]",
                "{\"a\":{\"b\":[1,{\"c\":null}]}}", "1\u{2028}", "\"\\ud83d\\ude00\"",
            ];
            let t = pick(r, TEXTS);
            let rev = pick(
                r,
                &[
                    "undefined",
                    "(k, v) => v",
                    "(k, v) => typeof v === 'number' ? v + 1 : v",
                    "function (k, v) { return k === 'a' ? undefined : v; }",
                    "function (k, v) { this.seen = (this.seen || '') + k + ';'; return v; }",
                    "(k, v) => Array.isArray(v) ? v.length : v",
                ],
            );
            vec![
                probe(),
                tpl(
                    "const s = $T;\nconsole.log(P(() => JSON.stringify(JSON.parse(s))), P(() => JSON.stringify(JSON.parse(s, $R))));",
                    &[("$T", &format!("{t:?}")), ("$R", rev)],
                ),
            ]
        }
        4 => vec![
            probe(),
            tpl(
                "const o = JSON.parse('$J');\nconsole.log(P(() => JSON.stringify(o)), P(() => Object.keys(o).join()), P(() => Object.getPrototypeOf(o) === Object.prototype), P(() => typeof o.__proto__));",
                &[(
                    "$J",
                    pick(
                        r,
                        &[
                            "{\"__proto__\":{\"polluted\":true},\"a\":1}",
                            "{\"a\":1,\"a\":2,\"b\":3}",
                            "{\"1\":1,\"0\":0,\"b\":2,\"a\":1}",
                            "[{\"x\":[]},{}]",
                            "{\"\":1}",
                            "{\"constructor\":{\"prototype\":1}}",
                        ],
                    ),
                )],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "console.log(P(() => JSON.stringify($V, null, $S)), P(() => JSON.stringify($V, $R, $S)));",
                &[
                    ("$V", pick(r, &["[]", "{}", "[[]]", "[{}]", "{ a: [] }", "{ a: {} }", "[1, [2, [3]]]", "{ a: 1, b: [1, { c: 2 }] }"])),
                    ("$S", pick(r, SPACES)),
                    ("$R", pick(r, REPLACERS)),
                ],
            ),
        ],
    }
}

// ── Number formatting ────────────────────────────────────────────────────────

/// `Number#toString(radix)` with fractions and negatives, `toFixed`,
/// `toPrecision`, `toExponential` at their rounding boundaries and range
/// errors, `Number(string)` / `parseFloat` / `parseInt` grammar.
pub fn gen_numfmt(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const NUMS: &[&str] = &[
        "0",
        "-0",
        "1",
        "-1",
        "0.5",
        "0.1",
        "0.2",
        "0.3",
        "0.7",
        "1.5",
        "2.5",
        "-2.5",
        "1.005",
        "1.255",
        "8.345",
        "10.235",
        "1.45",
        "1.55",
        "0.000001",
        "0.0000001",
        "123.456",
        "-123.456",
        "1e21",
        "1e-7",
        "1.7976931348623157e308",
        "5e-324",
        "2**53",
        "2**53 + 2",
        "255",
        "256",
        "0.000123",
        "99.99",
        "9.995",
        "0.045",
        "1234.5678",
        "1/3",
        "2/3",
        "100",
        "1e300",
        "3.14159",
        "-1e-10",
        "4.35",
        "0.615",
        "10.5",
        "NaN",
        "Infinity",
        "-Infinity",
        "0.1 + 0.2",
        "1e100",
        "123456789012345680000",
        "0.00001",
        "5e-7",
    ];
    let n = pick(r, NUMS);
    match r.below(7) {
        0 => vec![
            probe(),
            tpl(
                "console.log(P(() => ($N).toString($R)), P(() => ($N).toString($R2)), P(() => (-($N)).toString($R)));",
                &[
                    ("$N", n),
                    ("$R", pick(r, &["2", "3", "7", "8", "10", "16", "32", "36", "37", "1", "undefined", "'16'", "2.9"])),
                    ("$R2", pick(r, &["2", "5", "16", "36"])),
                ],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "console.log(P(() => ($N).toFixed($D)), P(() => ($N).toFixed($E)), P(() => (-($N)).toFixed($D)));",
                &[
                    ("$N", n),
                    ("$D", pick(r, &["0", "1", "2", "3", "5", "10", "20", "21", "100", "101", "-1", "undefined", "'2'", "NaN", "1.9"])),
                    ("$E", pick(r, &["0", "2", "4", "7"])),
                ],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "console.log(P(() => ($N).toPrecision($D)), P(() => ($N).toPrecision($E)), P(() => ($N).toPrecision()));",
                &[
                    ("$N", n),
                    ("$D", pick(r, &["1", "2", "3", "6", "10", "21", "22", "100", "101", "0", "-1", "undefined", "'4'", "1.9"])),
                    ("$E", pick(r, &["1", "2", "5", "7"])),
                ],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "console.log(P(() => ($N).toExponential($D)), P(() => ($N).toExponential($E)), P(() => ($N).toExponential()));",
                &[
                    ("$N", n),
                    ("$D", pick(r, &["0", "1", "2", "5", "20", "21", "100", "101", "-1", "undefined", "'3'", "1.9"])),
                    ("$E", pick(r, &["0", "2", "4"])),
                ],
            ),
        ],
        4 => {
            const STRS: &[&str] = &[
                "''", "' '", "'  12  '", "'12px'", "'1_000'", "'0x1f'", "'0X1F'", "'0b101'", "'0o17'",
                "'-0x1f'", "'+1'", "'1e3'", "'1e'", "'.5'", "'5.'", "'.'", "'Infinity'", "'-Infinity'",
                "'infinity'", "'+Infinity'", "'1,5'", "'\\n12\\t'", "'\\u00a012'", "'12\\u2028'", "'0.0000001'",
                "'1e-7'", "'123456789012345678901234567890'", "'0.1e-1'", "'00012'", "'-'", "'+'",
                "'1 2'", "'٣'", "'１２'", "'0x'", "'0b2'", "'9007199254740993'", "'1e1000'", "'-1e1000'",
            ];
            vec![
                probe(),
                tpl(
                    "const s = $S;\nconsole.log(P(() => Number(s)), P(() => +s), P(() => parseFloat(s)), P(() => parseInt(s)), P(() => parseInt(s, 16)), P(() => parseInt(s, 2)), P(() => parseInt(s, 36)), P(() => parseInt(s, 1)), P(() => parseInt(s, 37)));",
                    &[("$S", pick(r, STRS))],
                ),
            ]
        }
        5 => vec![
            probe(),
            tpl(
                "console.log(P(() => String($N)), P(() => ($N).toLocaleString()), P(() => ($N).toLocaleString('en-US', { maximumFractionDigits: $D })), P(() => Number.prototype.toString.call($N, 10)), P(() => ($N) + ''));",
                &[("$N", n), ("$D", pick(r, &["0", "1", "2", "3", "5"]))],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "console.log(P(() => Number.isInteger($N)), P(() => Number.isSafeInteger($N)), P(() => Math.round($N)), P(() => Math.trunc($N)), P(() => ($N) | 0), P(() => ($N) >>> 0), P(() => Number.parseFloat(String($N))), P(() => (($N) % 7)), P(() => Math.fround($N)));",
                &[("$N", n)],
            ),
        ],
    }
}

// ── String.prototype over Unicode ────────────────────────────────────────────

/// The `String.prototype` methods whose answer depends on Unicode data:
/// case mapping (special casing, final sigma), normalization, well-formedness,
/// whitespace trimming, `at`/`codePointAt`/`padStart` over surrogate pairs.
pub fn gen_unicode(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const STRS: &[&str] = &[
        "'stra\\u00dfe'",
        "'\\u0130stanbul'",
        "'\\u0049\\u0307'",
        "'\\u03a3\\u03a3'",
        "'\\u0391\\u03a3'",
        "'\\u0391\\u03a3 \\u0391'",
        "'\\u03c3\\u03c2'",
        "'\\u01c5'",
        "'\\u01c4\\u01c6'",
        "'\\ufb01nd'",
        "'\\u0149'",
        "'\\u1e9e'",
        "'\\u00e9\\u0301'",
        "'\\u00c5\\u212b\\u0041\\u030a'",
        "'\\ud835\\udcb3x'",
        "'\\ud83d\\ude00\\ud83c\\udf89'",
        "'\\u0041\\u0300\\u0323'",
        "'\\u1100\\u1161\\u11a8'",
        "'\\uac01'",
        "'\\u00a0\\u1680\\u2000\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000\\ufeff x \\u180e'",
        "'\\u200b x \\u200b'",
        "'\\ud801\\udc00'",
        "'\\ud801\\udc28'",
        "'\\u2126'",
        "'\\u0345'",
        "'\\u1f80'",
        "'\\u00b5'",
        "'\\u017f'",
        "'\\u212a'",
        "'\\uff21\\uff41'",
        "'i\\u0307'",
        "'\\u0587'",
        "'ǆ\\u01f0'",
    ];
    let s = pick(r, STRS);
    match r.below(8) {
        0 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => s.toUpperCase()), P(() => s.toLowerCase()), P(() => s.toLocaleUpperCase()), P(() => s.toLocaleLowerCase()), P(() => s.toUpperCase().length));",
                &[("$S", s)],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(['NFC','NFD','NFKC','NFKD'].map((f) => P(() => Array.from(s.normalize(f), (c) => c.codePointAt(0).toString(16)).join(' '))).join(' | '), P(() => s.normalize('nfc')), P(() => s.normalize(undefined).length));",
                &[("$S", s)],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => s.isWellFormed()), P(() => Array.from(s.toWellFormed(), (c) => c.codePointAt(0).toString(16)).join(' ')), P(() => encodeURIComponent(s)), P(() => escape(s)));",
                &[("$S", s)],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => JSON.stringify(s.trim())), P(() => JSON.stringify(s.trimStart())), P(() => JSON.stringify(s.trimEnd())), P(() => s.length));",
                &[("$S", s)],
            ),
        ],
        4 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => s.at($I)), P(() => s.codePointAt($I)), P(() => s.charCodeAt($I)), P(() => s[$I]), P(() => s.charAt($I)), P(() => s.slice($I, $J).length), P(() => s.substring($J, $I).length), P(() => s.substr($I, 2).length));",
                &[
                    ("$S", s),
                    ("$I", pick(r, &["0", "1", "2", "-1", "-2", "3", "10", "1.5", "NaN", "undefined"])),
                    ("$J", pick(r, &["0", "1", "2", "-1", "3", "undefined", "Infinity"])),
                ],
            ),
        ],
        5 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => s.padStart($N, $F).length), P(() => s.padEnd($N, $F).length), P(() => s.repeat(2).length), P(() => [...s].length), P(() => Array.from(s).length), P(() => s.split('').length), P(() => s.localeCompare($S2)), P(() => s.concat($S2).length));",
                &[
                    ("$S", s),
                    ("$N", pick(r, &["0", "1", "5", "8", "20"])),
                    ("$F", pick(r, &["' '", "'\\ud83d\\ude00'", "'ab'", "''", "undefined"])),
                    ("$S2", pick(r, STRS)),
                ],
            ),
        ],
        6 => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => s.indexOf($Q)), P(() => s.lastIndexOf($Q)), P(() => s.includes($Q)), P(() => s.startsWith($Q)), P(() => s.endsWith($Q)), P(() => s.search($Q)), P(() => s.split($Q).length), P(() => s.replace($Q, '[$&]').length), P(() => s.replaceAll($Q, 'X').length));",
                &[("$S", s), ("$Q", pick(r, &["'a'", "'\\u00e9'", "''", "'\\u0301'", "'\\u03c3'", "'\\u0049'", "'\\u00df'", "'x'"]))],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const s = $S;\nconsole.log(P(() => /./u.exec(s) && /./u.exec(s)[0].length), P(() => s.match(/\\w/giu) && s.match(/\\w/giu).length), P(() => /^[\\p{L}]+$/u.test(s)), P(() => /\\u{1f600}/u.test(s)), P(() => /[^x]/u.exec(s)[0].length), P(() => s.replace(/\\p{Lu}/gu, '_')), P(() => /\\u03c3/i.test(s)), P(() => /\\u212a/iu.test(s)));",
                &[("$S", s)],
            ),
        ],
    }
}

// ── Array holes, species, protocols ──────────────────────────────────────────

/// `Array.prototype` methods over holes and array-likes, `Symbol.species`,
/// `Symbol.isConcatSpreadable`, length edge cases, comparator contract.
pub fn gen_arrspecies(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const ARRS: &[&str] = &[
        "[1, , 3]",
        "[, , ,]",
        "[1, 2, , , 5, ,]",
        "[undefined, , null]",
        "new Array(3)",
        "Array(5).fill(0, 1, 3)",
        "[3, 1, , 2]",
        "[[1, [2]], , [3]]",
        "['b', , 'a', undefined, 'c']",
        "Object.assign([1, 2, 3], { 5: 6 })",
        "Object.assign([], { length: 3, 1: 'x' })",
    ];
    const METHODS: &[&str] = &[
        "map((x, i) => [x, i])",
        "filter(() => true)",
        "forEach((x, i) => { log.push(i); })",
        "some((x, i) => { log.push(i); return false; })",
        "every((x, i) => { log.push(i); return true; })",
        "reduce((a, x) => a + String(x), '')",
        "reduceRight((a, x) => a + String(x), '')",
        "indexOf(undefined)",
        "includes(undefined)",
        "findIndex((x) => x === undefined)",
        "findLastIndex((x) => x === undefined)",
        "keys().next().value",
        "entries().toArray().length",
        "flat().length",
        "flatMap((x) => [x]).length",
        "sort()",
        "sort((a, b) => 0)",
        "toSorted()",
        "toReversed().length",
        "toSpliced(0, 1).length",
        "with(0, 'w')",
        "reverse()",
        "join('-')",
        "toString()",
        "slice(1, 3)",
        "splice(1, 1)",
        "concat([9], [, 8])",
        "copyWithin(0, 1)",
        "fill('f', 1)",
        "at(-1)",
        "lastIndexOf(undefined)",
        "unshift(0)",
        "push(7)",
        "pop()",
        "shift()",
        "Object.keys(a).join()",
        "hasOwnProperty(1)",
        "length",
    ];
    let a = pick(r, ARRS);
    match r.below(5) {
        0 => {
            let m = pick(r, METHODS);
            let call = if m.starts_with("Object.") { m.to_string() } else { format!("a.{m}") };
            vec![
                probe(),
                "const log = [];".into(),
                tpl(
                    "const a = $A;\nconst show = (v) => Array.isArray(v) ? '[' + Array.from({ length: v.length }, (_, i) => i in v ? (Array.isArray(v[i]) ? show(v[i]) : String(v[i])) : '<hole>').join(',') + ']' : String(v);\nconsole.log(P(() => show($CALL)), show(a), log.join(','));",
                    &[("$A", a), ("$CALL", &call)],
                ),
            ]
        }
        1 => {
            let sp = pick(r, &["Array", "MyArr", "undefined", "null", "Object", "function () { return { length: 0 }; }", "function (n) { return { length: n }; }", "1"]);
            let m = pick(r, &["map((x) => x)", "filter(() => true)", "slice()", "splice(0, 1)", "concat([1])", "flat()", "flatMap((x) => [x])", "toSorted()"]);
            vec![
                probe(),
                tpl(
                    "class MyArr extends Array {}\nconst S = $SP;\nclass B extends Array { static get [Symbol.species]() { return S; } }\nconst b = B.from([1, 2, 3]);\nconsole.log(P(() => { const d = b.$M; return Object.prototype.toString.call(d) + ':' + (d instanceof B) + ':' + (d instanceof Array) + ':' + d.length + ':' + Object.keys(d).join(); }));",
                    &[("$SP", sp), ("$M", m)],
                ),
            ]
        }
        2 => vec![
            probe(),
            tpl(
                "const o = $O;\nconst spreadable = { length: 2, 0: 'x', 1: 'y', [Symbol.isConcatSpreadable]: true };\nconst notSpread = Object.assign([8, 9], { [Symbol.isConcatSpreadable]: false });\nconsole.log(P(() => [1].concat(spreadable, notSpread, o).map(String).join('|')), P(() => [1].concat(o).length));",
                &[("$O", pick(r, &["{ length: 1, 0: 'z' }", "'str'", "[, 1]", "new String('s')", "{ [Symbol.isConcatSpreadable]: true, length: 3 }", "[[]]", "(function () { return arguments; })(1, 2)"]))],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "const arrLike = $L;\nconsole.log(P(() => Array.prototype.map.call(arrLike, (x) => x + 1).join()), P(() => Array.from(arrLike).length), P(() => Array.prototype.slice.call(arrLike).length), P(() => Array.prototype.indexOf.call(arrLike, $X)), P(() => Array.prototype.join.call(arrLike)), P(() => Array.prototype.push.call(arrLike, 1)), P(() => Object.keys(arrLike).join()));",
                &[
                    ("$L", pick(r, &["{ length: 3, 0: 1, 2: 3 }", "{ length: '2', 0: 'a', 1: 'b' }", "{ length: -1 }", "{ length: 2 ** 32 }", "{ length: 1.9, 0: 4 }", "'abc'", "{ length: NaN, 0: 1 }", "{ 0: 1 }", "{ length: { valueOf() { return 2; } }, 0: 7, 1: 8 }"])),
                    ("$X", pick(r, &["1", "'a'", "undefined", "3"])),
                ],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const a = [$E];\nconst calls = [];\nconsole.log(P(() => a.sort((x, y) => { calls.push(1); return $CMP; }).join()), calls.length > 0, P(() => [..'zebra'].sort($C2).join('')), P(() => [3, 20, 100, 1].sort().join()), P(() => [true, false, null, undefined, 0, NaN, -1, 'a', 'B'].sort().map(String).join()));",
                &[
                    ("$E", pick(r, &["5, 1, 4", "'b', 'a', 'c'", "3, , 1", "2, undefined, 1", "10, 9, 1, 100"])),
                    ("$CMP", pick(r, &["x - y", "y - x", "0", "NaN", "'1'", "x > y ? 1 : -1", "undefined", "x < y"])),
                    ("$C2", pick(r, &["undefined", "(a, b) => b.localeCompare(a)", "(a, b) => (a < b) - (a > b)", "null"])),
                ],
            ),
        ],
    }
}

// ── Proxy / Reflect invariants ───────────────────────────────────────────────

/// Proxy traps that violate the object invariants (10.5), revocation, trap
/// receivers and arguments, and `Reflect` forwarding.
pub fn gen_proxyinv(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const SETUPS: &[&str] = &[
        "const t = {}; Object.defineProperty(t, 'k', { value: 1, writable: false, configurable: false });",
        "const t = {}; Object.defineProperty(t, 'k', { value: 1, writable: true, configurable: false });",
        "const t = { k: 1 }; Object.preventExtensions(t);",
        "const t = Object.freeze({ k: 1 });",
        "const t = { k: 1 };",
        "const t = {}; Object.defineProperty(t, 'k', { get() { return 1; }, configurable: false });",
        "const t = []; t.push(1, 2);",
        "const t = function f(a, b) {};",
    ];
    const TRAPS: &[&str] = &[
        "get(t, k, r) { return 2; }",
        "get(t, k, r) { return undefined; }",
        "set(t, k, v, r) { return true; }",
        "set(t, k, v, r) { return false; }",
        "has(t, k) { return false; }",
        "has(t, k) { return true; }",
        "deleteProperty(t, k) { return true; }",
        "deleteProperty(t, k) { return false; }",
        "defineProperty(t, k, d) { return true; }",
        "defineProperty(t, k, d) { return false; }",
        "getOwnPropertyDescriptor(t, k) { return undefined; }",
        "getOwnPropertyDescriptor(t, k) { return { value: 5, configurable: true }; }",
        "getOwnPropertyDescriptor(t, k) { return { value: 5, configurable: false }; }",
        "getOwnPropertyDescriptor(t, k) { return 1; }",
        "ownKeys(t) { return []; }",
        "ownKeys(t) { return ['k', 'k']; }",
        "ownKeys(t) { return ['k', 'extra']; }",
        "ownKeys(t) { return [1]; }",
        "ownKeys(t) { return 'k'; }",
        "getPrototypeOf(t) { return Array.prototype; }",
        "getPrototypeOf(t) { return null; }",
        "getPrototypeOf(t) { return 1; }",
        "setPrototypeOf(t, p) { return true; }",
        "setPrototypeOf(t, p) { return false; }",
        "isExtensible(t) { return !Reflect.isExtensible(t); }",
        "preventExtensions(t) { return true; }",
        "preventExtensions(t) { return false; }",
        "apply(t, th, args) { return args.length; }",
        "construct(t, args, nt) { return 1; }",
        "construct(t, args, nt) { return { made: args.length }; }",
    ];
    const OPS: &[&str] = &[
        "p.k",
        "p.other",
        "(p.k = 9, p.k)",
        "'k' in p",
        "'other' in p",
        "delete p.k",
        "delete p.other",
        "Object.keys(p).join()",
        "Reflect.ownKeys(p).join()",
        "JSON.stringify(Object.getOwnPropertyDescriptor(p, 'k'))",
        "Object.defineProperty(p, 'k', { value: 3 }) === p",
        "Reflect.defineProperty(p, 'k', { value: 3 })",
        "Object.getPrototypeOf(p) === Object.prototype",
        "Reflect.setPrototypeOf(p, null)",
        "Object.setPrototypeOf(p, {}) === p",
        "Object.isExtensible(p)",
        "Object.preventExtensions(p) === p",
        "Reflect.preventExtensions(p)",
        "p(1, 2)",
        "new p(1, 2)",
        "Object.isFrozen(p)",
        "Object.entries(p).length",
        "Array.isArray(p)",
        "p.length",
        "Object.prototype.toString.call(p)",
    ];
    match r.below(3) {
        0 => vec![
            probe(),
            tpl(
                "$S\nconst log = [];\nconst p = new Proxy(t, { $T });\nconsole.log(P(() => $O), P(() => $O2));",
                &[
                    ("$S", pick(r, SETUPS)),
                    ("$T", pick(r, TRAPS)),
                    ("$O", pick(r, OPS)),
                    ("$O2", pick(r, OPS)),
                ],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const log = [];\nconst h = new Proxy({}, { get(_, trap) { log.push(trap); return undefined; } });\nconst p = new Proxy($T, h);\n$OP;\nconsole.log(log.join(','));",
                &[
                    ("$T", pick(r, &["{ a: 1 }", "[1, 2]", "function () {}", "class C {}", "new Map()"])),
                    (
                        "$OP",
                        pick(
                            r,
                            &[
                                "try { p.a; 'a' in p; delete p.a; p.b = 1; } catch (e) {}",
                                "try { Object.keys(p); } catch (e) {}",
                                "try { Object.entries(p); } catch (e) {}",
                                "try { for (const k in p) {} } catch (e) {}",
                                "try { JSON.stringify(p); } catch (e) {}",
                                "try { ({ ...p }); } catch (e) {}",
                                "try { Object.assign({}, p); } catch (e) {}",
                                "try { [...p]; } catch (e) {}",
                                "try { Object.freeze(p); } catch (e) {}",
                                "try { Object.isFrozen(p); } catch (e) {}",
                                "try { p instanceof Object; } catch (e) {}",
                                "try { Array.isArray(p); } catch (e) {}",
                                "try { Object.getOwnPropertyNames(p); } catch (e) {}",
                                "try { p(); } catch (e) {}",
                                "try { new p(); } catch (e) {}",
                            ],
                        ),
                    ),
                ],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const { proxy, revoke } = Proxy.revocable($T, {});\nconst before = P(() => $OP);\nrevoke(); revoke();\nconsole.log(before, P(() => $OP), P(() => typeof proxy), P(() => Array.isArray(proxy)), P(() => Object.prototype.toString.call(proxy)));",
                &[
                    ("$T", pick(r, &["{ a: 1 }", "[1]", "function () { return 1; }"])),
                    ("$OP", pick(r, &["proxy.a", "proxy()", "'a' in proxy", "Object.keys(proxy).length", "proxy.length", "typeof proxy", "new Proxy(proxy, {})"])),
                ],
            ),
        ],
    }
}

// ── Async ordering ───────────────────────────────────────────────────────────

/// The order in which microtasks, `process.nextTick`, `await`, thenable
/// assimilation, async generators and timers of different delays run.
pub fn gen_asyncorder(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    match r.below(7) {
        0 => vec![
            "const out = [];".into(),
            tpl(
                "Promise.resolve().then(() => out.push('p1')).then(() => out.push('p2'));\nprocess.nextTick(() => out.push('t1'));\nqueueMicrotask(() => out.push('q1'));\n(async () => { out.push('a0'); await null; out.push('a1'); await $AW; out.push('a2'); })();\nPromise.resolve().then(() => out.push('p3'));\nprocess.nextTick(() => out.push('t2'));\nsetTimeout(() => console.log(out.join(' ')), 5);",
                &[("$AW", pick(r, &["undefined", "Promise.resolve()", "{ then(f) { f(); } }", "new Promise((res) => res())", "(async () => {})()"]))],
            ),
        ],
        1 => vec![
            "const out = [];".into(),
            tpl(
                "const thenable = { then(res) { out.push('then-called'); res('tv'); } };\nconst p = Promise.resolve(thenable);\nout.push('sync-after-resolve');\np.then((v) => out.push('got ' + v));\nPromise.resolve().then(() => out.push('m1')).then(() => out.push('m2')).then(() => out.push('m3'));\nnew Promise((res) => res($RES)).then((v) => out.push('r:' + (typeof v)));\nsetTimeout(() => console.log(out.join(' | ')), 5);",
                &[("$RES", pick(r, &["thenable", "Promise.resolve(1)", "1", "Promise.reject(2).catch(() => 3)"]))],
            ),
        ],
        2 => vec![
            "const out = [];".into(),
            tpl(
                "setTimeout(() => out.push('t$A'), $A);\nsetTimeout(() => out.push('t$B'), $B);\nsetTimeout(() => { out.push('t0'); Promise.resolve().then(() => out.push('m-in-t0')); process.nextTick(() => out.push('nt-in-t0')); }, 0);\nsetTimeout(() => out.push('t0b'), 0);\nsetTimeout(() => out.push('t1'), 1);\nsetTimeout(() => console.log(out.join(' ')), 30);",
                &[("$A", pick(r, &["5", "10", "2"])), ("$B", pick(r, &["3", "12", "7", "5"]))],
            ),
        ],
        3 => vec![
            "const out = [];".into(),
            tpl(
                "async function* g() { out.push('g-start'); yield 1; out.push('g-mid'); yield Promise.resolve(2); out.push('g-end'); return $RET; }\n(async () => {\n  for await (const v of g()) out.push('v' + v);\n  out.push('done');\n  const it = g();\n  out.push(JSON.stringify(await it.next()));\n  out.push(JSON.stringify(await it.return('early')));\n  out.push(JSON.stringify(await it.next()));\n})();\nPromise.resolve().then(() => out.push('side1')).then(() => out.push('side2'));\nsetTimeout(() => console.log(out.join(' ')), 10);",
                &[("$RET", pick(r, &["7", "undefined", "Promise.resolve(8)"]))],
            ),
        ],
        4 => vec![
            "const out = [];".into(),
            tpl(
                "const f = async (x) => { out.push('f' + x); if (x === 2) throw new Error('boom'); return x; };\n(async () => {\n  try { await Promise.all([f(1), f(2), f(3)]); } catch (e) { out.push('caught ' + e.message); }\n  const s = await Promise.allSettled([f(1), f(2)]);\n  out.push(s.map((x) => x.status).join());\n  out.push(String(await Promise.race([f(4), new Promise(() => {})])));\n  out.push(String(await Promise.any([f(2), f(5)])));\n  try { await Promise.any([f(2)]); } catch (e) { out.push(e.constructor.name + ':' + e.errors.length); }\n  out.push('end');\n})();\nsetTimeout(() => console.log(out.join(' ')), 10);",
                &[],
            ),
        ],
        5 => vec![
            "const out = [];".into(),
            tpl(
                "const p = new Promise((res, rej) => { out.push('exec'); $BODY });\np.then((v) => out.push('then:' + v), (e) => out.push('rej:' + e)).finally(() => out.push('fin')).then((v) => out.push('after:' + v));\np.catch((e) => out.push('catch:' + e));\nout.push('sync-end');\nsetTimeout(() => console.log(out.join(' ')), 5);",
                &[("$BODY", pick(r, &["res(1); rej(2);", "rej(1); res(2);", "throw 3;", "res(); res(4);", "res(Promise.reject(5));", "res(new Promise((r) => r(6)));", "queueMicrotask(() => res(7));"]))],
            ),
        ],
        _ => vec![
            "const out = [];".into(),
            tpl(
                "let n = 0;\nconst tick = () => { if (++n < 4) { process.nextTick(tick); Promise.resolve().then(() => out.push('p' + n)); } out.push('n' + n); };\nprocess.nextTick(tick);\nPromise.resolve().then(() => out.push('first'));\nsetImmediate(() => out.push('imm'));\nsetTimeout(() => { out.push('to'); console.log(out.join(' ')); }, $D);",
                &[("$D", pick(r, &["20", "30"]))],
            ),
        ],
    }
}

// ── Class fields, private names, static blocks ───────────────────────────────

/// Initialization order of fields, private methods/accessors, static blocks and
/// computed keys across inheritance; `#x in o` brand checks.
pub fn gen_classfield(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    match r.below(7) {
        0 => vec![
            probe(),
            tpl(
                "const log = [];\nconst key = (n) => { log.push('key:' + n); return n; };\nclass A {\n  [key('a')] = log.push('init a');\n  static [key('s')] = log.push('static s');\n  static { log.push('block1'); }\n  [key('b')]() {}\n  static { log.push('block2 ' + this.name); }\n  c = log.push('init c');\n  constructor() { log.push('ctor A'); }\n}\nlog.push('defined');\nnew A();\nclass B extends A { d = log.push('init d'); constructor() { log.push('B before'); super(); log.push('B after'); } }\nnew B();\nconsole.log(log.join(' | '));",
                &[],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "class P {\n  #v = $INIT;\n  #m() { return this.#v + 1; }\n  get #acc() { return this.#v * 2; }\n  set #acc(x) { this.#v = x; }\n  static #sp = 'sp';\n  static #sm() { return P.#sp; }\n  run() { this.#acc = 5; return [this.#m(), this.#acc, P.#sm(), #v in this, #m in this, #acc in this]; }\n  static has(o) { return #v in o; }\n}\nconsole.log(P(() => new P().run().join()), P(() => P.has(new P())), P(() => P.has({})), P(() => P.has(Object.create(new P()))), P(() => P.has(1)));",
                &[("$INIT", pick(r, &["1", "'s'", "[]", "0"]))],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "class A { #x = 1; static read(o) { return o.#x; } static write(o) { o.#x = 2; } static call(o) { return o.#m(); } #m() { return 1; } }\nconst fake = {};\nconst child = Object.create(new A());\nconst tests = [() => A.read(fake), () => A.write(fake), () => A.call(fake), () => A.read(child), () => A.read(null), () => A.read(new A()), () => A.read($X)];\nconsole.log(tests.map((f) => P(f)).join(' ; '));",
                &[("$X", pick(r, &["1", "'s'", "undefined", "function () {}"]))],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "const log = [];\nclass A { x = (log.push('A.x'), 1); constructor() { log.push('A ctor sees ' + this.constructor.name + ' ' + this.y); } }\nclass B extends A { y = (log.push('B.y'), 2); z = this.y + this.x; }\nconst b = new B();\nconsole.log(log.join(' | '), JSON.stringify(b), Object.getOwnPropertyNames(b).join());",
                &[],
            ),
        ],
        4 => vec![
            probe(),
            tpl(
                "class A { static x = 1; static y = this.x + 1; static f = () => this.y; static g() { return this.x; } static { this.z = this.f() + 1; } }\nclass B extends A { static x = 10; static { this.w = super.g() + this.z; } }\nconsole.log(A.x, A.y, A.f(), A.z, B.x, B.y, B.z, B.w, Object.getOwnPropertyNames(B).join(), Object.keys(A).join());",
                &[],
            ),
        ],
        5 => vec![
            probe(),
            tpl(
                "class A { 'quoted' = 1; 42 = 2; [Symbol.for('s')] = 3; static 'sq' = 4; get a() { return 1; } set a(v) {} static get sa() { return 's'; } #p = 5; }\nconst a = new A();\nconsole.log(Object.getOwnPropertyNames(a).join(), Reflect.ownKeys(a).length, Object.getOwnPropertyNames(A).join(), Object.getOwnPropertyNames(A.prototype).join(), JSON.stringify(Object.getOwnPropertyDescriptor(A.prototype, 'a') && Object.keys(Object.getOwnPropertyDescriptor(A.prototype, 'a'))));",
                &[],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const log = [];\nclass A { constructor() { log.push('A:' + new.target.name); } }\nclass B extends A { f = log.push('B.f'); constructor() { const t = () => this; log.push('pre'); $BODY } }\nconsole.log(P(() => { new B(); return log.join(' | '); }));",
                &[("$BODY", pick(r, &["super(); log.push('post');", "log.push(String(typeof this));", "super(); super();", "return 1;", "return { custom: 1 };", "const f = () => super(); f(); log.push('arrow');"]))],
            ),
        ],
    }
}

// ── Destructuring and iterator closing ───────────────────────────────────────

/// When iterators are closed (`return()` called) by destructuring, `for-of`,
/// spread, `yield*`, `Array.from`, and when an abrupt close error is swallowed.
pub fn gen_iterclose(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let iter = "const log = [];\nconst mk = (n, opts = {}) => ({ [Symbol.iterator]() { let i = 0; return { next() { log.push('next' + i); if (opts.throwNext && i === opts.throwNext) throw new Error('nx'); return i < n ? { value: i++, done: false } : { value: undefined, done: true }; }, return(v) { log.push('return'); if (opts.throwReturn) throw new Error('rt'); return opts.badReturn ? 1 : { done: true }; } }; } });";
    match r.below(7) {
        0 => vec![
            probe(),
            iter.to_string(),
            tpl(
                "console.log(P(() => { const $PAT = mk($N, { throwReturn: $TR }); return 'ok'; }), log.join(' '));",
                &[
                    ("$PAT", pick(r, &["[a]", "[a, b]", "[a, ...rest]", "[, ]", "[a, b, c, d, e]", "[]", "[a = 5]", "[[x]]"])),
                    ("$N", pick(r, &["0", "1", "2", "5"])),
                    ("$TR", pick(r, &["false", "true"])),
                ],
            ),
        ],
        1 => vec![
            probe(),
            iter.to_string(),
            tpl(
                "console.log(P(() => { for (const x of mk($N, { throwReturn: $TR, badReturn: $BR })) { $BODY } return 'done'; }), log.join(' '));",
                &[
                    ("$N", pick(r, &["3", "1"])),
                    ("$TR", pick(r, &["false", "true"])),
                    ("$BR", pick(r, &["false", "true"])),
                    ("$BODY", pick(r, &["break;", "continue;", "if (x === 1) break;", "throw new Error('body');", "if (x === 0) return 'early';", "if (x === 1) continue;"])),
                ],
            ),
        ],
        2 => vec![
            probe(),
            iter.to_string(),
            tpl(
                "console.log(P(() => { const [a, b] = mk($N, { throwNext: $TN }); return a + ',' + b; }), log.join(' '));\nlog.length = 0;\nconsole.log(P(() => [...mk(3, { throwNext: $TN })].length), log.join(' '));\nlog.length = 0;\nconsole.log(P(() => Array.from(mk(3, { throwNext: $TN }), (x) => { if (x === 1) throw new Error('map'); return x; }).length), log.join(' '));",
                &[("$N", pick(r, &["5", "1"])), ("$TN", pick(r, &["1", "2", "0"]))],
            ),
        ],
        3 => vec![
            probe(),
            iter.to_string(),
            tpl(
                "function* g() { try { yield* mk(3, { throwReturn: $TR }); } finally { log.push('g-finally'); } }\nconst it = g();\nit.next();\nconsole.log(P(() => JSON.stringify($CALL)), log.join(' '));",
                &[
                    ("$TR", pick(r, &["false", "true"])),
                    ("$CALL", pick(r, &["it.return(9)", "it.throw(new Error('t'))", "it.next()", "[...g()].length"])),
                ],
            ),
        ],
        4 => vec![
            probe(),
            "const log = [];".into(),
            tpl(
                "function* g() { try { log.push('start'); const x = yield 1; log.push('got ' + x); yield 2; } catch (e) { log.push('catch ' + e); yield 'c'; } finally { log.push('finally'); $FIN } }\nconst it = g();\nconst steps = [() => it.next('a'), () => it.next('b'), () => it.throw('E'), () => it.return('R'), () => it.next()];\nconsole.log(steps.map((s) => P(() => JSON.stringify(s()))).join(' '), log.join(' '));",
                &[("$FIN", pick(r, &["", "yield 'f';", "return 'override';", "throw new Error('fin');"]))],
            ),
        ],
        5 => vec![
            probe(),
            "const log = [];".into(),
            tpl(
                "const src = { get a() { log.push('get a'); return undefined; }, get b() { log.push('get b'); return 2; }, get c() { log.push('get c'); return 3; } };\nconst d = (n, v) => { log.push('default ' + n); return v; };\nconsole.log(P(() => { const { a = d('a', 1), c, [d('key', 'b')]: b = d('b', 0), ...rest } = src; return [a, b, c, Object.keys(rest).join()].join(); }), log.join(' | '));\nlog.length = 0;\nconsole.log(P(() => { let x, y; [x = d('x', 1), y = d('y', 2)] = [undefined, null]; ({ p: x, q: y = d('q', 3) } = { p: 4 }); return [x, y].join(); }), log.join(' | '));",
                &[],
            ),
        ],
        _ => vec![
            probe(),
            "const log = [];".into(),
            tpl(
                "const it = { [Symbol.iterator]() { return { next() { log.push('next'); return $RES; }, return() { log.push('return'); return {}; } }; } };\nconsole.log(P(() => { const [a] = it; return String(a); }), P(() => [...it].length), P(() => { for (const x of it) break; return 'b'; }), log.join(' '));",
                &[("$RES", pick(r, &["1", "null", "{ done: true }", "{ get done() { log.push('done'); return true; }, get value() { log.push('value'); return 7; } }", "undefined"]))],
            ),
        ],
    }
}

// ── Completion values ────────────────────────────────────────────────────────

/// The completion value `eval` reports for statement lists: `if`, loops,
/// `switch`, labelled blocks, `try`/`finally`, `break`/`continue` and
/// declarations (14.x `UpdateEmpty` rules).
pub fn gen_completion(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const PROGS: &[&str] = &[
        "1; if (true) {}",
        "1; if (false) 2;",
        "1; if (true) { 2; } else { 3; }",
        "1; do { 2; break; } while (false)",
        "1; do { 2; continue; } while (false)",
        "1; while (false) 2;",
        "1; for (var i = 0; i < 3; i++) { i; }",
        "1; for (var i = 0; i < 3; i++) { if (i == 1) break; i; }",
        "1; for (var i = 0; i < 3; i++) { if (i == 1) continue; i + 10; }",
        "1; for (var k in {a: 1, b: 2}) { k; }",
        "1; for (var v of [5, 6]) { v; }",
        "1; switch (1) { case 1: 2; break; case 2: 3; }",
        "1; switch (1) { case 0: 2; default: 3; }",
        "1; switch (1) { case 1: }",
        "1; switch (3) { case 1: 2; }",
        "1; a: { 2; break a; 3; }",
        "1; a: { break a; }",
        "1; a: 2;",
        "1; try { 2; } finally { 3; }",
        "1; try { 2; } catch (e) { 3; }",
        "1; try { throw 0; } catch (e) { 3; }",
        "1; try { throw 0; } catch (e) { } finally { 4; }",
        "1; try { } finally { 4; }",
        "1; do { try { 2; break; } finally { 3; } } while (false)",
        "1; do { 2; try { break; } finally { 3; } } while (false)",
        "1; do { 2; try { 5; } finally { break; } } while (false)",
        "1; l: do { 2; try { 5; } finally { break l; } } while (true)",
        "1; { 2; { 3; } }",
        "1; { }",
        "1; ;",
        "1; var x = 2;",
        "1; let y = 2;",
        "1; function f() {}",
        "1; class C {}",
        "1; with ({}) { 2; }",
        "1; with ({}) ;",
        "2; if (1) ; else 3;",
        "var i = 0; while (i < 3) { i++; if (i == 2) break; i * 10; }",
        "1; (function () { 2; })()",
        "1; 2 ? 3 : 4;",
        "x = 5",
        "typeof 1; void 0;",
        "switch (1) { default: 7; case 2: 8; }",
        "1; do { 2; } while (false); ;",
    ];
    let a = pick(r, PROGS);
    let b = pick(r, PROGS);
    vec![
        probe(),
        tpl(
            "for (const src of [$A, $B]) console.log(JSON.stringify(src), P(() => { const v = (0, eval)(src); return typeof v === 'undefined' ? 'undefined' : JSON.stringify(v); }));",
            &[("$A", &format!("{a:?}")), ("$B", &format!("{b:?}"))],
        ),
    ]
}

// ── Accessors and descriptors on prototype chains ────────────────────────────

/// Getters/setters and data properties found on a prototype: assignment through
/// the chain, receivers, `super` property access, enumeration order and
/// shadowing, `Object.defineProperty` on the prototype after instantiation.
pub fn gen_protoget(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    match r.below(6) {
        0 => vec![
            probe(),
            tpl(
                "const proto = { $DEF };\nconst o = Object.create(proto);\nconst res = [];\nres.push(P(() => (o.k = 5, o.k)));\nres.push(Object.prototype.hasOwnProperty.call(o, 'k'));\nres.push(P(() => proto.k));\nres.push(P(() => { 'use strict'; o.k = 6; return o.k; }));\nconsole.log(res.join(' '));",
                &[(
                    "$DEF",
                    pick(
                        r,
                        &[
                            "k: 1",
                            "get k() { return 'g'; }",
                            "set k(v) { this._k = v; }",
                            "get k() { return this._k; }, set k(v) { this._k = v * 2; }",
                        ],
                    ),
                )],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const proto = Object.defineProperty({}, 'k', { value: 1, writable: $W, configurable: true, enumerable: true });\nconst o = Object.create(proto);\nconsole.log(P(() => (o.k = 2, o.k)), Object.keys(o).join(), P(() => { 'use strict'; o.k = 3; return o.k; }), P(() => Reflect.set(o, 'k', 4)), P(() => Object.getOwnPropertyDescriptor(o, 'k') && 'own'));",
                &[("$W", pick(r, &["true", "false"]))],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "class A { get v() { return 'A.v:' + this.tag; } set v(x) { this.tag = 'set' + x; } static get s() { return this.name; } m() { return 'A.m'; } }\nclass B extends A { get v() { return 'B>' + super.v; } set v(x) { super.v = x + '!'; } static get s() { return 'B>' + super.s; } m() { return 'B>' + super.m(); } }\nconst b = new B();\nb.tag = 't';\nconsole.log(b.v, (b.v = 'x', b.tag), B.s, b.m(), P(() => Object.getOwnPropertyDescriptor(B.prototype, 'v').set.call({}, 1)));",
                &[],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "function F() { this.own = 1; }\nF.prototype.shared = 's';\nF.prototype.fn = function () { return 'fn'; };\nconst a = new F();\nconst b = new F();\nObject.defineProperty(F.prototype, 'late', { get() { return 'late:' + this.own; }, enumerable: $E, configurable: true });\nconst keys = [];\nfor (const k in a) keys.push(k);\nconsole.log(keys.join(), a.late, Object.keys(a).join(), JSON.stringify(a), 'late' in b, a.hasOwnProperty('late'), Object.entries(a).length, JSON.stringify(Object.assign({}, a)));",
                &[("$E", pick(r, &["true", "false"]))],
            ),
        ],
        4 => vec![
            probe(),
            tpl(
                "const base = { get x() { return this.v; }, set x(n) { this.v = n; }, v: 'base' };\nconst mid = Object.create(base, { v: { value: 'mid', writable: true, configurable: true } });\nconst leaf = Object.create(mid);\nconsole.log(leaf.x, (leaf.x = 'L', leaf.v), Object.keys(leaf).join(), mid.v, base.v, P(() => Reflect.get(base, 'x', leaf)), P(() => Reflect.set(base, 'x', 'R', leaf)), leaf.v, Object.keys(leaf).join());",
                &[],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const o = { get a() { return 1; }, b: 2 };\nconst copy = { ...o };\nconst assigned = Object.assign({}, o);\nconst desc = Object.getOwnPropertyDescriptors(o);\nconsole.log(JSON.stringify(Object.getOwnPropertyDescriptor(copy, 'a')), typeof Object.getOwnPropertyDescriptor(desc.a, 'get'), Object.keys(desc).join(), JSON.stringify(Object.getOwnPropertyDescriptor(o, 'a').set), P(() => Object.defineProperties({}, desc).a), P(() => Object.create(null, desc).b));",
                &[],
            ),
        ],
    }
}

// ── Symbol protocols ─────────────────────────────────────────────────────────

/// `Symbol.toPrimitive`, `toStringTag`, `hasInstance`, `isConcatSpreadable`,
/// `match`/`replace`/`search`/`split`/`matchAll`, `iterator` and `asyncIterator`
/// hooks, `Symbol.for`/`keyFor`/`description`.
pub fn gen_symproto(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    match r.below(7) {
        0 => vec![
            probe(),
            tpl(
                "const o = { [Symbol.toStringTag]: $TAG };\nconsole.log(P(() => Object.prototype.toString.call(o)), P(() => String(o)), P(() => `${o}`), P(() => Object.prototype.toString.call(new (class X { get [Symbol.toStringTag]() { return 'XT'; } })())), P(() => Object.prototype.toString.call(Object.assign(new Map(), { [Symbol.toStringTag]: 'M2' }))), P(() => String(Object.create(null, { [Symbol.toStringTag]: { value: 'N' } }))));",
                &[("$TAG", pick(r, &["'Tagged'", "1", "undefined", "{}", "''", "Symbol('s')", "null"]))],
            ),
        ],
        1 => vec![
            probe(),
            tpl(
                "const mk = (v) => ({ [Symbol.match](s) { return ['m', s, v]; }, [Symbol.replace](s, r) { return 'rep:' + s + ':' + r; }, [Symbol.search](s) { return 77; }, [Symbol.split](s, l) { return ['sp', s, l]; }, [Symbol.matchAll](s) { return ['ma', s][Symbol.iterator](); }, toString() { return 'STR'; } });\nconst re = mk(1);\nconsole.log(P(() => 'abc'.match(re)), P(() => 'abc'.replace(re, 'X')), P(() => 'abc'.search(re)), P(() => 'abc'.split(re, 3)), P(() => [...'abc'.matchAll(re)].join()), P(() => 'abc'.replaceAll(re, 'Y')), P(() => 'abc'.startsWith(re)), P(() => 'abc'.includes(re)));",
                &[],
            ),
        ],
        2 => vec![
            probe(),
            tpl(
                "const re = /a/;\nre[Symbol.match] = $M;\nconsole.log(P(() => 'a/a/'.startsWith(re)), P(() => 'a/a/'.endsWith(re)), P(() => 'a/a/'.includes(re)), P(() => '/a/'.includes(Object.assign(/a/, { [Symbol.match]: false }))), P(() => RegExp(re) === re), P(() => new RegExp(re) === re), P(() => RegExp({ [Symbol.match]: true, source: 'q', flags: 'g', constructor: RegExp }).flags));",
                &[("$M", pick(r, &["false", "true", "undefined", "0", "null"]))],
            ),
        ],
        3 => vec![
            probe(),
            tpl(
                "const d = Symbol($DESC);\nconst s = Symbol.for('app.key');\nconsole.log(P(() => d.toString()), P(() => d.description), P(() => Symbol.keyFor(d)), P(() => Symbol.keyFor(s)), P(() => Symbol.for('app.key') === s), P(() => Symbol.keyFor('x')), P(() => String(Symbol.iterator)), P(() => Symbol.iterator.description), P(() => `${d}`), P(() => d + ''), P(() => Object(d) == d), P(() => typeof Object(d)), P(() => JSON.stringify({ [d]: 1, k: d })), P(() => Object.getOwnPropertySymbols({ [d]: 1 }).length), P(() => new Symbol()));",
                &[("$DESC", pick(r, &["'desc'", "''", "undefined", "1", "null", "{}"]))],
            ),
        ],
        4 => vec![
            probe(),
            tpl(
                "const prim = { [Symbol.toPrimitive]: $F };\nconsole.log(P(() => +prim), P(() => prim + ''), P(() => `${prim}`), P(() => prim * 2), P(() => prim == 1), P(() => String(prim)), P(() => [prim] + ''), P(() => new Date(prim).getTime()), P(() => Number(prim)), P(() => prim < 2), P(() => ({ [prim]: 1 })));",
                &[("$F", pick(r, &["(h) => h === 'number' ? 1 : h === 'string' ? 's' : 'd'", "() => 5", "undefined", "null", "7", "function () { return Symbol('x'); }", "() => { throw new RangeError('tp'); }"]))],
            ),
        ],
        5 => vec![
            probe(),
            tpl(
                "class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }\nfunction F() {}\nObject.defineProperty(F, Symbol.hasInstance, { value: () => $V });\nconst bound = (function () {}).bind(null);\nconsole.log(P(() => 2 instanceof Even), P(() => 3 instanceof Even), P(() => ({}) instanceof F), P(() => ({}) instanceof bound), P(() => ({}) instanceof { [Symbol.hasInstance]: () => 1 }), P(() => ({}) instanceof {}), P(() => Function.prototype[Symbol.hasInstance].call(Even, 4)), P(() => ({}) instanceof (() => {})));",
                &[("$V", pick(r, &["true", "false", "1", "'s'", "undefined"]))],
            ),
        ],
        _ => vec![
            probe(),
            tpl(
                "const log = [];\nconst it = { [Symbol.iterator]: $IT };\nconsole.log(P(() => [...it].join()), P(() => Array.from(it).join()), P(() => new Set(it).size), P(() => Math.max(...it)), P(() => Object.fromEntries(it) && 1), P(() => { const [a] = it; return a; }), P(() => new Map(it).size));",
                &[("$IT", pick(r, &["function* () { yield 1; yield 2; }", "() => [3, 4][Symbol.iterator]()", "() => ({ next: () => ({ done: true }) })", "() => 1", "undefined", "null", "function () { return 'ab'[Symbol.iterator](); }"]))],
            ),
        ],
    }
}
