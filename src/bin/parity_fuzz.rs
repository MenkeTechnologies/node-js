//! Differential parity fuzzer: `node -e <s>` vs our `node -e <s>`.
//!
//! Generates thousands of grammar-driven, deterministic-output JavaScript
//! snippets, runs each through both interpreters, and reports every case where
//! stdout OR success/failure diverge. Each case is produced from a per-index seed
//! so any divergence replays exactly: `parity-fuzz --seed <N> --once`.
//!
//! The generator is biased toward the historically weak areas of a from-scratch
//! JavaScript engine (float `repr` and the exponential-notation threshold,
//! ToInt32/ToUint32 bitwise wrap, the `==` coercion matrix, `+` operand
//! coercion, string/array methods with negative/OOB indices, `toFixed`/
//! `toPrecision` rounding, JSON round-trips). Pure random bytes only produce
//! mutual SyntaxErrors that agree on both sides and teach nothing.
//!
//! Determinism invariant: the generator NEVER emits a construct whose output is
//! nondeterministic for reasons unrelated to parity (`Math.random`, `Date`,
//! `performance.now`, `Symbol()` identity, object addresses, unstably-ordered
//! iteration). Every probe is wrapped in `console.log`, and object/array
//! iteration order is insertion order on both sides, so every reported divergence
//! is a real parity gap, not a false positive.
//!
//! Scope invariant: the generator only emits constructs node-js actually
//! implements. It does NOT emit `class`, generators (`function*`/`yield`),
//! `async`/`await`, regex literals, `Map`/`Set`/`Promise`, `BigInt` (`10n`), or
//! `instanceof` — those are known-unimplemented, so generating them would flood
//! the report with "not built yet" noise instead of finding real bugs in shipped
//! features. They are recorded as known gaps in BUGS.md, not fuzzed.
//!
//! Subprocess-only: this binary never links the nodejs library — it compares two
//! `node` processes, exactly as a user would observe them.
//!
//! Build:  cargo build --bin parity-fuzz
//! Run:    ./target/debug/parity-fuzz --count 5000

use std::io::Read as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Deterministic PRNG (splitmix64) — no `rand` dependency.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

fn pick<'a, T>(rng: &mut Rng, xs: &'a [T]) -> &'a T {
    &xs[rng.below(xs.len() as u64) as usize]
}

// ---------------------------------------------------------------------------
// Binary resolution / invocation
// ---------------------------------------------------------------------------

/// Our `node` binary — the sibling of this harness binary.
fn ours_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_node") {
        return PathBuf::from(p);
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        let cand = d.join("node");
        if cand.exists() {
            return cand;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("node")
}

/// The ORACLE — reference Node.js. Every divergence this harness reports is
/// "node-js disagrees with THIS interpreter", so which interpreter it is, is part
/// of the result: Node versions differ (e.g. error wording, some `util.inspect`
/// formatting), so a baseline is only meaningful against the Node that produced
/// it. `NODE_JS_FUZZ_NODE` names the oracle explicitly; if it is set but unusable
/// this is a HARD ERROR — silently falling back to a different Node would answer a
/// different question than the one that was asked.
fn resolve_oracle() -> String {
    if let Ok(p) = std::env::var("NODE_JS_FUZZ_NODE") {
        if version_of(&p).is_none() {
            eprintln!("parity-fuzz: NODE_JS_FUZZ_NODE={p}: not a usable node");
            std::process::exit(2);
        }
        return p;
    }
    for p in ["node", "/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"] {
        if version_of(p).is_some() {
            return p.to_string();
        }
    }
    eprintln!("parity-fuzz: no reference node found; set NODE_JS_FUZZ_NODE");
    std::process::exit(2);
}

/// `<prog> --version` output, or None if the program can't be run.
fn version_of(prog: &str) -> Option<String> {
    let o = Command::new(prog).arg("--version").output().ok()?;
    if !o.status.success() && o.stdout.is_empty() && o.stderr.is_empty() {
        return None;
    }
    let mut s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if s.is_empty() {
        s = String::from_utf8_lossy(&o.stderr).trim().to_string();
    }
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `<path> (<version>)`, for the run header and the report file, so a divergence
/// record can be attributed to the exact oracle that produced it.
fn oracle_id(oracle: &str) -> String {
    let v = version_of(oracle).unwrap_or_else(|| "unknown".to_string());
    format!("{oracle} ({v})")
}

static CMP_STDERR: AtomicBool = AtomicBool::new(false);

/// Raw bytes, never `String`: an interpreter legitimately emits output that is
/// not valid UTF-8. `read_to_string` FAILS on such a stream and leaves the
/// buffer empty, so both sides would report "" and silently agree — a
/// divergence the harness could never see. Comparing bytes (and only ever
/// lossy-rendering for the human report) keeps the byte surface honest.
struct RunOut {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit: i32,
    timed_out: bool,
}

/// Render captured bytes for a report. Invalid UTF-8 is shown lossily AND
/// followed by a hex line — two different invalid byte strings both render to
/// U+FFFD, so without the hex the record would show a divergence as identical
/// text.
fn render(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches('\n');
    if std::str::from_utf8(bytes).is_err() {
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        return format!("{text}\n  (hex) {}", hex.join(" "));
    }
    text.to_string()
}

/// Best-effort stderr normalization for `--stderr`: Node prints a multi-line
/// stack trace, node-js its own terse format, so we collapse to the last
/// non-empty line (usually `ErrorType: message`) lowercased. Cross-interpreter
/// stderr rarely matches verbatim; this is a loose "same error class" check.
fn norm_stderr(s: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(s);
    let last = text
        .lines()
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_lowercase();
    last.into_bytes()
}

/// A parity gap: stdout bytes differ, OR one side accepted the program (exit 0)
/// while the other rejected it. We compare success-ness, not the exact exit
/// code — a from-scratch interpreter is free to pick its own nonzero code for
/// an uncaught exception, so "both rejected it" is agreement, not a gap.
fn differs(oracle: &RunOut, ours: &RunOut) -> bool {
    if (oracle.exit == 0) != (ours.exit == 0) {
        return true;
    }
    if oracle.stdout != ours.stdout {
        return true;
    }
    if CMP_STDERR.load(Ordering::Relaxed)
        && norm_stderr(&oracle.stderr) != norm_stderr(&ours.stderr)
    {
        return true;
    }
    false
}

/// Run `<prog> -e <src>` with a wall-clock timeout enforced by a watchdog: two
/// reader threads drain stdout/stderr (so a large writer can't deadlock on a
/// full pipe) while the main thread polls `try_wait` and `kill()`s on overrun.
fn run_prog(prog: &Path, src: &str, timeout: Duration) -> RunOut {
    let mut cmd = Command::new(prog);
    cmd.arg("-e")
        .arg(src)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return RunOut {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit: -1,
                timed_out: false,
            }
        }
    };

    let mut out_h = child.stdout.take().map(|mut o| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = o.read_to_end(&mut b);
            b
        })
    });
    let mut err_h = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = e.read_to_end(&mut b);
            b
        })
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let exit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit = status.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let s = child.wait().ok();
                    exit = s.and_then(|s| s.code()).unwrap_or(-1);
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => {
                exit = -1;
                break;
            }
        }
    }

    let stdout = out_h.take().and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err_h.take().and_then(|h| h.join().ok()).unwrap_or_default();
    RunOut {
        stdout,
        stderr,
        exit,
        timed_out,
    }
}

fn build_program(stmts: &[String]) -> String {
    stmts.join("\n")
}

// ---------------------------------------------------------------------------
// Generators — each returns a statement list whose stdout is deterministic.
//
// Only constructs node-js implements are emitted (see the scope invariant in the
// module doc). Every probe expression is wrapped in `console.log(...)`.
// ---------------------------------------------------------------------------

const INTS: &[&str] = &[
    "0", "1", "2", "3", "5", "7", "10", "-1", "-3", "-7", "42", "100", "-100", "255", "1000",
];
const POSINTS: &[&str] = &["1", "2", "3", "4", "5", "6", "8", "10"];
// Floats biased toward the exponential-notation threshold and repr edge cases.
const FLOATS: &[&str] = &[
    "0.1", "0.2", "0.5", "1.5", "2.5", "3.14", "10.0", "-1.5", "100.0", "0.0", "-0.0", "1e21",
    "1e-7", "1e-6", "1e100", "1.5e300", "123.456", "0.0001234", "9.999999e20", "1e22", "-0.4",
    "1.005", "8.575",
];
// Values that exercise NaN / Infinity / -0 propagation.
const SPECIALS: &[&str] = &["NaN", "Infinity", "-Infinity", "0/0", "1/0", "-1/0", "-0"];
const STRS: &[&str] = &[
    "'hello'",
    "'World'",
    "'abc'",
    "'JavaScript'",
    "''",
    "'a'",
    "'foo bar'",
    "'  pad  '",
    "'AbC'",
    "'café'",
];

/// Number formatting / float repr — the historically weakest surface of a
/// from-scratch JS number printer (exponential threshold, `toFixed`/`toPrecision`
/// rounding, `-0`, NaN/Infinity arithmetic).
fn gen_num(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, FLOATS);
    let b = pick(r, FLOATS);
    let k = r.below(7); // 0..6 fractional/precision digits
    let e = match r.below(11) {
        0 => "0.1 + 0.2".to_string(),
        1 => format!("{a} + {b}"),
        2 => format!("{a} * {b}"),
        3 => format!("{a} / {b}"),
        4 => format!("({a}).toFixed({k})"),
        5 => format!("({a}).toPrecision({})", 1 + r.below(6)),
        // toString(radix) on an INTEGER receiver: fractional-radix expansion is a
        // documented known gap (BUGS.md), so keep this to well-defined integers.
        6 => format!("({}).toString({})", pick(r, INTS), 2 + r.below(35)),
        7 => pick(r, SPECIALS).to_string(),
        8 => format!("{} + {}", pick(r, SPECIALS), a),
        9 => a.to_string(),
        _ => format!("{a} - {b} + {a}"),
    };
    vec![format!("console.log({e})")]
}

/// Integer / bitwise — `& | ^ ~ << >> >>>` with JS ToInt32/ToUint32 semantics and
/// the 32-bit wrap on large operands.
fn gen_bitwise(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let big = &[
        "0xffffffff",
        "0x7fffffff",
        "0x80000000",
        "4294967296",
        "-1",
        "2147483648",
        "255",
        "16",
        "1",
        "0",
        "-256",
    ];
    let a = pick(r, big);
    let b = pick(r, &["0", "1", "4", "8", "16", "31", "32", "33"]);
    let op = pick(r, &["&", "|", "^", "<<", ">>", ">>>"]);
    let e = match r.below(6) {
        0 => format!("{a} {op} {b}"),
        1 => format!("~{a}"),
        2 => format!("{a} >>> {b}"),
        3 => format!("{a} << {b}"),
        4 => format!("({a} & {}) | {b}", pick(r, big)),
        _ => format!("{a} ^ {}", pick(r, big)),
    };
    vec![format!("console.log({e})")]
}

/// Equality coercion matrix — `==`/`!=` across number/string/bool/null/undefined
/// pairs vs `===`.
fn gen_equality(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let vals = &[
        "0", "1", "''", "'0'", "'1'", "'a'", "null", "undefined", "true", "false", "[]", "[0]",
        "NaN",
    ];
    let a = pick(r, vals);
    let b = pick(r, vals);
    let op = pick(r, &["==", "!=", "===", "!=="]);
    let e = match r.below(4) {
        0 => format!("{a} {op} {b}"),
        1 => format!("{a} == {b}"),
        2 => format!("{a} === {b}"),
        _ => format!("({a} == {b}) === ({b} == {a})"),
    };
    vec![format!("console.log({e})")]
}

/// String methods — slicing with negative/OOB indices, pad/repeat/split/replace,
/// and template-literal interpolation of mixed types.
fn gen_strmeth(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let s = pick(r, STRS);
    let idx = &["0", "1", "2", "-1", "-2", "3", "10"];
    let a = pick(r, idx);
    let b = pick(r, idx);
    let e = match r.below(14) {
        0 => format!("{s}.slice({a}, {b})"),
        1 => format!("{s}.substring({a}, {b})"),
        2 => format!("{s}.substr({a}, {b})"),
        3 => format!("{s}.toUpperCase()"),
        4 => format!("{s}.padStart({}, '*')", r.below(8)),
        5 => format!("{s}.padEnd({}, '.')", r.below(8)),
        6 => format!("{s}.repeat({})", r.below(4)),
        // split('') on a SHORT string: a >6-element array triggers node's
        // util.inspect multi-line grouping, a documented known gap (BUGS.md), so
        // keep the result within the single-line regime.
        7 => format!("{}.split('')", pick(r, &["'abc'", "'a'", "''", "'hi'", "'abcde'", "'Wor'"])),
        8 => format!("{s}.replace('a', 'X')"),
        9 => format!("{s}.replaceAll('a', 'X')"),
        10 => format!("{s}.indexOf('o')"),
        11 => format!("{s}.at({a})"),
        12 => format!("`[${{{s}}}]-[${{{}}}]`", pick(r, INTS)),
        _ => format!("`${{{}}}${{{}}}`", pick(r, INTS), s),
    };
    vec![format!("console.log({e})")]
}

/// Array methods — `map/filter/reduce/slice/splice/sort/flat/flatMap/join/concat`
/// with mixed element types (default lexicographic sort is a classic gotcha),
/// plus spread.
fn gen_array(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let arr = pick(
        r,
        &[
            "[3, 1, 2, 5, 4]",
            "[10, 9, 100, 20, 1]",
            "[1, 2, 3]",
            "['b', 'a', 'c']",
            "[[1, 2], [3], [4, 5]]",
            "[1, 'a', true, null]",
        ],
    );
    let e = match r.below(14) {
        0 => format!("{arr}.map(x => x)"),
        1 => "[3, 1, 2, 5, 4].sort()".to_string(),
        2 => "[10, 9, 100, 20, 1].sort()".to_string(),
        3 => "[3, 1, 2, 5, 4].sort((a, b) => a - b)".to_string(),
        4 => format!("{arr}.slice({}, {})", r.below(5), 1 + r.below(5)),
        5 => format!("{arr}.join('-')"),
        // concat adds a single element so the result stays ≤6 (see split note).
        6 => format!("{arr}.concat([9])"),
        7 => format!("{arr}.indexOf({})", pick(r, INTS)),
        8 => format!("{arr}.includes({})", pick(r, INTS)),
        9 => "[[1, 2], [3], [4, 5]].flat()".to_string(),
        10 => "[1, 2, 3].flatMap(x => [x, x * 2])".to_string(),
        11 => "[1, 2, 3, 4].filter(x => x % 2 === 0)".to_string(),
        12 => "[1, 2, 3, 4].reduce((a, b) => a + b, 0)".to_string(),
        _ => format!("[...{arr}, 7]"),
    };
    vec![format!("console.log({e})")]
}

/// Coercion in `+` — string+number, array+number, object+string. The array/object
/// `ToPrimitive` path is a classic from-scratch gap.
fn gen_plus(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let vals = &[
        "1", "2", "'a'", "'x'", "[1, 2]", "[3]", "[]", "true", "null", "3.5", "'5'",
    ];
    let a = pick(r, vals);
    let b = pick(r, vals);
    let e = match r.below(5) {
        0 => format!("{a} + {b}"),
        1 => format!("{a} + {b} + {}", pick(r, vals)),
        2 => format!("[1, 2] + {}", pick(r, INTS)),
        3 => format!("'' + {a}"),
        _ => format!("{a} + '' + {b}"),
    };
    vec![format!("console.log({e})")]
}

/// JSON round-trips — `JSON.stringify` of nested structures with and without an
/// indent argument, plus a `parse`→`stringify` round-trip.
fn gen_json(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let obj = pick(
        r,
        &[
            "{a: 1, b: [2, 3], c: 'x'}",
            "{name: 'Ada', nums: [1, 2, 3], ok: true}",
            "[1, [2, [3, [4]]]]",
            "{nested: {deep: {value: 42}}, list: []}",
            "{n: null, u: undefined, f: false, s: 'hi'}",
        ],
    );
    let e = match r.below(4) {
        0 => format!("JSON.stringify({obj})"),
        1 => format!("JSON.stringify({obj}, null, 2)"),
        2 => format!("JSON.stringify({obj}, null, '  ')"),
        _ => format!("JSON.stringify(JSON.parse(JSON.stringify({obj})))"),
    };
    vec![format!("console.log({e})")]
}

/// typeof / truthiness / ternary / logical — `typeof` of each value kind,
/// `&&`/`||`/`??` value-returning chains, `!!x`, ternary nesting.
fn gen_logic(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let vals = &[
        "0", "1", "''", "'x'", "null", "undefined", "true", "false", "[]", "{}", "NaN",
    ];
    let a = pick(r, vals);
    let b = pick(r, vals);
    let e = match r.below(7) {
        0 => format!("typeof {a}"),
        1 => format!("{a} && {b}"),
        2 => format!("{a} || {b}"),
        3 => format!("{a} ?? {b}"),
        4 => format!("!!{a}"),
        5 => format!("{a} ? {b} : {}", pick(r, vals)),
        _ => format!("typeof ({a} || {b})"),
    };
    vec![format!("console.log({e})")]
}

/// parseInt / parseFloat / Number — radix, leading/trailing junk, `0x` prefixes,
/// `Number()` coercion of odd strings.
fn gen_parse(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let strs = &[
        "'42'", "'42px'", "'  17  '", "'0x1f'", "'101'", "'3.14'", "'1e3'", "''", "'abc'",
        "'-5'", "'  '", "'0.5e2'", "'Infinity'", "'  12.5abc'",
    ];
    let s = pick(r, strs);
    let e = match r.below(6) {
        0 => format!("parseInt({s})"),
        1 => format!("parseInt({s}, {})", pick(r, &["2", "8", "10", "16"])),
        2 => format!("parseFloat({s})"),
        3 => format!("Number({s})"),
        4 => "parseInt('101', 2)".to_string(),
        _ => format!("Number({s}) + 1"),
    };
    vec![format!("console.log({e})")]
}

/// Object — `Object.keys/values/entries/assign/fromEntries`, computed keys, and
/// key insertion order in output.
fn gen_object(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let obj = pick(
        r,
        &[
            "{b: 2, a: 1, c: 3}",
            "{x: 10, y: 20}",
            "{name: 'Ada', age: 36}",
            "{}",
        ],
    );
    let e = match r.below(8) {
        0 => format!("Object.keys({obj})"),
        1 => format!("Object.values({obj})"),
        2 => format!("Object.entries({obj})"),
        3 => format!("Object.assign({{}}, {obj}, {{d: 4}})"),
        4 => "Object.fromEntries([['a', 1], ['b', 2]])".to_string(),
        5 => format!("({{...{obj}, z: 9}})"),
        6 => "({['k' + 1]: 'v', ['k' + 2]: 'w'})".to_string(),
        _ => obj.to_string(),
    };
    vec![format!("console.log({e})")]
}

/// Arithmetic — precedence, `%`/`**`, unary minus, mixed int/float.
fn gen_arith(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, INTS);
    let b = pick(r, INTS);
    let c = pick(r, INTS);
    let exp = pick(r, &["2", "3", "0", "-1"]);
    let op = pick(r, &["+", "-", "*", "/", "%"]);
    let e = match r.below(6) {
        0 => format!("{a} {op} {b}"),
        1 => format!("{a} + {b} * {c}"),
        2 => format!("({a} + {b}) * {c}"),
        3 => format!("(-{a}) ** {exp}"),
        4 => format!("{a} % {b} + {c}"),
        _ => format!("{a} {op} {b} {op} {c}"),
    };
    vec![format!("console.log({e})")]
}

/// Math.* — the deterministic subset (never `Math.random`). Trig/log outputs are
/// full-precision f64 and print identically when the repr is correct.
fn gen_math(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let x = pick(r, INTS);
    let f = pick(r, FLOATS);
    let e = match r.below(12) {
        0 => format!("Math.floor({f})"),
        1 => format!("Math.ceil({f})"),
        2 => format!("Math.round({f})"),
        3 => format!("Math.trunc({f})"),
        4 => format!("Math.abs({x})"),
        5 => format!("Math.sign({x})"),
        6 => format!("Math.max({x}, {})", pick(r, INTS)),
        7 => format!("Math.min({x}, {})", pick(r, INTS)),
        8 => format!("Math.pow({}, {})", pick(r, POSINTS), pick(r, &["2", "3", "0"])),
        9 => format!("Math.sqrt({})", pick(r, POSINTS)),
        10 => "Math.max(...[3, 1, 4, 1, 5])".to_string(),
        _ => format!("Math.hypot({}, {})", pick(r, POSINTS), pick(r, POSINTS)),
    };
    vec![format!("console.log({e})")]
}

/// Control flow — loops/closures accumulating a deterministic value, exercising
/// the compiler's statement lowering rather than a single expression.
fn gen_control(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let n = 3 + r.below(6);
    match r.below(4) {
        0 => vec![
            "let s = 0;".into(),
            format!("for (let i = 0; i < {n}; i++) s += i;"),
            "console.log(s);".into(),
        ],
        1 => vec![
            "const out = [];".into(),
            format!("for (const v of [1, 2, 3, 4]) out.push(v * {n});"),
            "console.log(out);".into(),
        ],
        2 => vec![
            format!("const f = x => x < 2 ? x : f(x - 1) + f(x - 2);"),
            format!("console.log(f({}));", 5 + r.below(8)),
        ],
        _ => vec![
            "let acc = 1;".into(),
            format!("let i = 1; while (i <= {n}) {{ acc *= i; i++; }}"),
            "console.log(acc);".into(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Mode dispatch
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Mode {
    Mixed,
    Num,
    Bitwise,
    Equality,
    Strmeth,
    Array,
    Plus,
    Json,
    Logic,
    Parse,
    Object,
    Arith,
    Math,
    Control,
}

const REAL_MODES: &[Mode] = &[
    Mode::Num,
    Mode::Bitwise,
    Mode::Equality,
    Mode::Strmeth,
    Mode::Array,
    Mode::Plus,
    Mode::Json,
    Mode::Logic,
    Mode::Parse,
    Mode::Object,
    Mode::Arith,
    Mode::Math,
    Mode::Control,
];

/// Generate the statement list for a seed in the selected mode. `Mixed` rotates
/// across every real mode by seed, so a plain run exercises the whole surface.
fn gen_case(seed: u64, mode: Mode) -> Vec<String> {
    match mode {
        Mode::Mixed => {
            let m = REAL_MODES[(seed % REAL_MODES.len() as u64) as usize];
            gen_case(seed, m)
        }
        Mode::Num => gen_num(seed),
        Mode::Bitwise => gen_bitwise(seed),
        Mode::Equality => gen_equality(seed),
        Mode::Strmeth => gen_strmeth(seed),
        Mode::Array => gen_array(seed),
        Mode::Plus => gen_plus(seed),
        Mode::Json => gen_json(seed),
        Mode::Logic => gen_logic(seed),
        Mode::Parse => gen_parse(seed),
        Mode::Object => gen_object(seed),
        Mode::Arith => gen_arith(seed),
        Mode::Math => gen_math(seed),
        Mode::Control => gen_control(seed),
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Mixed => "mixed",
        Mode::Num => "num",
        Mode::Bitwise => "bitwise",
        Mode::Equality => "equality",
        Mode::Strmeth => "strmeth",
        Mode::Array => "array",
        Mode::Plus => "plus",
        Mode::Json => "json",
        Mode::Logic => "logic",
        Mode::Parse => "parse",
        Mode::Object => "object",
        Mode::Arith => "arith",
        Mode::Math => "math",
        Mode::Control => "control",
    }
}

const ALL_MODES: &[Mode] = &[
    Mode::Mixed,
    Mode::Num,
    Mode::Bitwise,
    Mode::Equality,
    Mode::Strmeth,
    Mode::Array,
    Mode::Plus,
    Mode::Json,
    Mode::Logic,
    Mode::Parse,
    Mode::Object,
    Mode::Arith,
    Mode::Math,
    Mode::Control,
];

fn mode_from_name(s: &str) -> Option<Mode> {
    ALL_MODES.iter().copied().find(|&m| mode_name(m) == s)
}

// ---------------------------------------------------------------------------
// Divergence check + delta-debug minimizer
// ---------------------------------------------------------------------------

fn diverges(script: &str, bin: &Path, oracle: &str, timeout: Duration) -> bool {
    let o = run_prog(Path::new(oracle), script, timeout);
    let r = run_prog(bin, script, timeout);
    !o.timed_out && differs(&o, &r)
}

/// Delta-debug: greedily drop statements while the divergence survives.
fn minimize(stmts: Vec<String>, bin: &Path, oracle: &str, timeout: Duration) -> Vec<String> {
    let mut cur = stmts;
    let mut changed = true;
    while changed && cur.len() > 1 {
        changed = false;
        for i in 0..cur.len() {
            let mut cand = cur.clone();
            cand.remove(i);
            if cand.is_empty() {
                continue;
            }
            if diverges(&build_program(&cand), bin, oracle, timeout) {
                cur = cand;
                changed = true;
                break;
            }
        }
    }
    cur
}

/// Normalize a minimal reproducer to a stable gap-class signature: mask numeric
/// literals and quoted words so many instances of the same gap collapse to one
/// signature. Used by `--baseline` so known gaps don't fail CI but new ones do.
fn signature(program: &str) -> String {
    let body = program
        .lines()
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    mask_words(&mask_numbers(&body))
}

/// Replace every quoted string literal ('...' or "...") with `W`.
fn mask_words(s: &str) -> String {
    let bytes: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            i += 1; // closing quote
            out.push('W');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Replace every run of digits (with an optional leading `-` and a `.` fraction)
/// with `N`.
fn mask_numbers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_alnum = out
            .chars()
            .last()
            .map(|p| p.is_alphanumeric() || p == '_')
            .unwrap_or(false);
        if c.is_ascii_digit() && !prev_alnum {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            out.push('N');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    count: u64,
    base_seed: u64,
    once: bool,
    timeout_ms: u64,
    out_path: PathBuf,
    max_report: usize,
    jobs: usize,
    mode: Mode,
    verify: usize,
    baseline: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut count = 2000u64;
    let mut base_seed = 1u64;
    let mut once = false;
    let mut timeout_ms = 5000u64;
    let mut max_report = 200usize;
    let mut mode = Mode::Mixed;
    let mut verify = 1usize;
    let mut baseline: Option<PathBuf> = None;
    let mut jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-fuzz")
        .join("divergences.txt");

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--count" | "-c" => {
                i += 1;
                count = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(count);
            }
            "--seed" | "-s" => {
                i += 1;
                base_seed = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(base_seed);
            }
            "--once" => once = true,
            "--timeout-ms" => {
                i += 1;
                timeout_ms = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(timeout_ms);
            }
            "--out" | "-o" => {
                i += 1;
                if let Some(p) = argv.get(i) {
                    out_path = PathBuf::from(p);
                }
            }
            "--max-report" => {
                i += 1;
                max_report = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(max_report);
            }
            "--jobs" | "-j" => {
                i += 1;
                jobs = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .filter(|&j| j >= 1)
                    .unwrap_or(jobs);
            }
            "--mode" | "-m" => {
                i += 1;
                match argv.get(i).and_then(|s| mode_from_name(s)) {
                    Some(m) => mode = m,
                    None => {
                        eprintln!(
                            "unknown --mode '{}'",
                            argv.get(i).map(|s| s.as_str()).unwrap_or("")
                        );
                        std::process::exit(2);
                    }
                }
            }
            a if a.starts_with("--") && mode_from_name(&a[2..]).is_some() => {
                mode = mode_from_name(&a[2..]).unwrap();
            }
            "--verify" => {
                i += 1;
                verify = argv
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .filter(|&k| k >= 1)
                    .unwrap_or(verify);
            }
            "--baseline" => {
                i += 1;
                baseline = argv.get(i).map(PathBuf::from);
            }
            "--stderr" => {
                CMP_STDERR.store(true, Ordering::Relaxed);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }
    Args {
        count,
        base_seed,
        once,
        timeout_ms,
        out_path,
        max_report,
        jobs,
        mode,
        verify,
        baseline,
    }
}

fn print_help() {
    eprintln!(
        "parity-fuzz — differential node/node-js parity fuzzer\n\
         \n\
         --count N        number of cases (default 2000)\n\
         --seed N         base seed; case i uses seed+i (default 1)\n\
         --mode M         mixed (default; rotates all modes), num, bitwise,\n\
         equality, strmeth, array, plus, json, logic, parse,\n\
         object, arith, math, control\n\
         (each also accepted as a `--<mode>` shorthand)\n\
         --stderr         also require the normalized error line to match\n\
         --once           run a single case (seed) and print both outputs\n\
         --timeout-ms N   per-process wall-clock timeout (default 5000)\n\
         --out PATH       divergence corpus file\n\
         --max-report N   stop after N divergences (default 200)\n\
         --jobs N         parallel workers (default = CPU count)\n\
         --verify K       require K consecutive divergences to report (default 1)\n\
         --baseline FILE  allowlist of known-gap signatures; only a NEW\n\
         divergence (not in FILE) fails the run (exit 1)\n\
         \n\
         env  NODE_JS_FUZZ_NODE=PATH  the reference Node to compare against\n\
         (HARD ERROR if set but unusable). Every run prints the oracle it used."
    );
}

fn main() {
    let args = parse_args();
    let bin = ours_bin();
    let oracle = resolve_oracle();
    let timeout = Duration::from_millis(args.timeout_ms);

    if !bin.exists() {
        eprintln!(
            "node-js `node` binary not found at {}; run `cargo build` first",
            bin.display()
        );
        std::process::exit(2);
    }

    // --once: replay a single seed, minimize if it diverges, dump both sides.
    if args.once {
        let stmts = gen_case(args.base_seed, args.mode);
        let script = build_program(&stmts);
        let o = run_prog(Path::new(&oracle), &script, timeout);
        let r = run_prog(&bin, &script, timeout);
        let diverged = !o.timed_out && differs(&o, &r);
        println!("seed   : {}", args.base_seed);
        println!("mode   : {}", mode_name(args.mode));
        let (show, o, r) = if diverged && stmts.len() > 1 {
            let m = minimize(stmts, &bin, &oracle, timeout);
            let ms = build_program(&m);
            let mo = run_prog(Path::new(&oracle), &ms, timeout);
            let mr = run_prog(&bin, &ms, timeout);
            (ms, mo, mr)
        } else {
            (script, o, r)
        };
        println!("program:\n  {}", show.replace('\n', "\n  "));
        println!("--- node exit={} timeout={} ---", o.exit, o.timed_out);
        let _ = std::io::stdout().write_all(&o.stdout);
        println!("--- node-js exit={} timeout={} ---", r.exit, r.timed_out);
        let _ = std::io::stdout().write_all(&r.stdout);
        println!("--- {} ---", if diverged { "DIVERGE" } else { "match" });
        std::process::exit(if diverged { 1 } else { 0 });
    }

    let next = AtomicU64::new(0);
    let checked = AtomicU64::new(0);
    let timeouts = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let divergences: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
    let start = Instant::now();

    eprintln!("oracle: {}", oracle_id(&oracle));
    eprintln!("ours  : {}", bin.display());
    eprintln!(
        "fuzzing {} cases ({}) across {} workers…",
        args.count,
        mode_name(args.mode),
        args.jobs
    );

    std::thread::scope(|scope| {
        for _ in 0..args.jobs {
            scope.spawn(|| loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= args.count {
                    break;
                }
                let seed = args.base_seed.wrapping_add(idx);
                let stmts = gen_case(seed, args.mode);
                let script = build_program(&stmts);
                let o = run_prog(Path::new(&oracle), &script, timeout);
                let r = run_prog(&bin, &script, timeout);
                let done = checked.fetch_add(1, Ordering::Relaxed) + 1;
                if o.timed_out || r.timed_out {
                    timeouts.fetch_add(1, Ordering::Relaxed);
                }
                // oracle-side timeout ⇒ pathological case; not a parity gap.
                if !o.timed_out && differs(&o, &r) {
                    let minimal = minimize(stmts, &bin, &oracle, timeout);
                    let mscript = build_program(&minimal);
                    let mo = run_prog(Path::new(&oracle), &mscript, timeout);
                    let mr = run_prog(&bin, &mscript, timeout);
                    // Re-verify: a REAL gap diverges every time; a transient
                    // (empty output under resource pressure) won't reproduce.
                    let mut confirmed = differs(&mo, &mr);
                    for _ in 1..args.verify.max(1) {
                        if !confirmed {
                            break;
                        }
                        confirmed = diverges(&mscript, &bin, &oracle, timeout);
                    }
                    if !confirmed {
                        continue;
                    }
                    let err_of = |o: &RunOut| -> String {
                        if CMP_STDERR.load(Ordering::Relaxed) {
                            format!(
                                "\n  stderr: {}",
                                render(&norm_stderr(&o.stderr)).replace('\n', "\n  ")
                            )
                        } else {
                            String::new()
                        }
                    };
                    let rec = format!(
                        "==== seed {seed} ====\n\
                         program:\n  {}\n\
                         node    : exit={} timeout={}{}\n{}\n\
                         node-js : exit={} timeout={}{}\n{}\n",
                        mscript.replace('\n', "\n  "),
                        mo.exit,
                        mo.timed_out,
                        err_of(&mo),
                        render(&mo.stdout),
                        mr.exit,
                        mr.timed_out,
                        err_of(&mr),
                        render(&mr.stdout),
                    );
                    let mut d = divergences.lock().unwrap();
                    d.push((seed, rec));
                    if d.len() >= args.max_report {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                if done % 500 == 0 {
                    let n = divergences.lock().unwrap().len();
                    eprintln!(
                        "  {done}/{} checked, {n} divergences, {:.0}/s",
                        args.count,
                        done as f64 / start.elapsed().as_secs_f64().max(0.001)
                    );
                }
            });
        }
    });

    let checked = checked.load(Ordering::Relaxed);
    let timeouts = timeouts.load(Ordering::Relaxed);
    let mut divergences: Vec<(u64, String)> = divergences.into_inner().unwrap();
    divergences.sort_by_key(|(seed, _)| *seed);
    let divergences: Vec<String> = divergences.into_iter().map(|(_, r)| r).collect();
    let elapsed = start.elapsed();

    let sig_of = |rec: &str| -> String {
        let prog = rec
            .split("program:\n")
            .nth(1)
            .and_then(|s| s.split("\nnode ").next())
            .unwrap_or(rec);
        signature(prog)
    };

    let allowed: std::collections::HashSet<String> = match &args.baseline {
        Some(bp) => std::fs::read_to_string(bp)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect(),
        None => std::collections::HashSet::new(),
    };
    let mut new_records: Vec<&String> = Vec::new();
    let mut new_sigs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut known = 0usize;
    for rec in &divergences {
        let sig = sig_of(rec);
        if args.baseline.is_some() && allowed.contains(&sig) {
            known += 1;
        } else {
            new_records.push(rec);
            new_sigs.insert(sig);
        }
    }

    let oracle = oracle_id(&oracle);
    println!(
        "\nfuzzed {checked} cases in {:.1}s ({:.0}/s)\n\
         oracle      : {}\n\
         divergences : {} ({} known / {} new)\n\
         timeouts    : {}",
        elapsed.as_secs_f64(),
        checked as f64 / elapsed.as_secs_f64().max(0.001),
        oracle,
        divergences.len(),
        known,
        new_records.len(),
        timeouts,
    );

    if !divergences.is_empty() {
        if let Some(parent) = args.out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::File::create(&args.out_path) {
            let _ = writeln!(f, "# oracle: {oracle}");
            for d in &divergences {
                let _ = writeln!(f, "{d}");
            }
            println!(
                "wrote {} divergences to {}",
                divergences.len(),
                args.out_path.display()
            );
        }
    }

    if !new_records.is_empty() {
        println!(
            "\n--- {} NEW gap signature(s) (add to baseline once triaged) ---",
            new_sigs.len()
        );
        for s in &new_sigs {
            println!("{s}");
        }
        println!(
            "\n--- first {} new divergence record(s) ---",
            new_records.len().min(5)
        );
        for d in new_records.iter().take(5) {
            println!("{d}");
        }
        std::process::exit(1);
    }
    if known > 0 {
        println!("all {known} divergences are known (in baseline) — OK");
    }
}
