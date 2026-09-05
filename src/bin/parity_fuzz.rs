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
//! nondeterministic for reasons unrelated to parity — `Math.random`,
//! `performance.now`, `Date.now`, `Symbol()` identity, object addresses,
//! unstably-ordered iteration. A FIXED `Date` epoch is fine and is emitted
//! (`new Date(0)`, `new Date(86400000)`, `structuredClone(new Date(…))`); what
//! is excluded is the reading of a clock, not the type. The environment is
//! pinned too — both children run with `TZ=UTC` and `LANG=LC_ALL=en_US.UTF-8`
//! rather than inheriting the developer's, since reference `node` is not
//! locale- or TZ-invariant. Every probe is wrapped in `console.log` or observed
//! through the exit status, and object/array iteration order is insertion order
//! on both sides, so every reported divergence is a real parity gap, not a false
//! positive.
//!
//! Scope invariant: the generator only emits constructs node-js actually
//! implements. The implemented surface now includes ES6 `class`
//! (fields/methods/inheritance/`super`/statics/getters/setters), the prototype
//! chain + `instanceof`, generators (`function*`/`yield`/`yield*`),
//! `Map`/`Set`/`WeakMap`/`WeakSet`, `Symbol`/`Symbol.iterator`, Promises +
//! `async`/`await` + the microtask/timer event loop, `BigInt` (`10n` arithmetic,
//! mixing-`TypeError`, comparisons, formatting), and regular expressions (the
//! Rust-`regex`-supported subset: char classes, quantifiers, anchors, groups,
//! alternation, `\d\w\s`, flags `g`/`i`/`m` — NOT backreferences/lookaround) —
//! all exercised by the `class`, `generator`, `mapset`, `proto`, `async`,
//! `bigint`, and `regex` modes. The `regex` mode emits the subset the engine can
//! represent (see BUGS.md for the exact set and the divergences). It does not
//! generate backreferences or lookaround — not because those are rejected, which
//! was true of the old `regex`-crate engine and is no longer: the engine is
//! `fancy-regex`, and `/(a)\1/`, `/(?=a)a/`, `/a(?!b)/` and `/(?<=a)b/` all
//! execute correctly today. They stay out of the generator because the
//! REMAINING divergences live there (a JS forward reference such as `/\1(a)/`
//! matches the empty string in JS and fails here), and a generator that emitted
//! them would report the same known gap on every run. The async mode emits ONLY
//! deterministically-schedulable output (fixed microtask/timer ordering,
//! resolved-value chains) — never wall-clock- or identity-dependent results.
//!
//! The `descriptor`, `freeze`, `identity`, `clone` and `error` modes cover the
//! object model: the `writable`/`enumerable`/`configurable` triple and every
//! reader that must agree about it, `freeze`/`seal`/`preventExtensions` and
//! their sloppy-mode write and `delete` outcomes, builtin singleton identity and
//! prototype-chain reads, `structuredClone`'s reference graph, and error
//! own-property shape (`cause`, `AggregateError`, subclassing). `identity`
//! prints only fixed booleans and brand strings, never an address, and `error`
//! never prints stack text — its frames are engine-specific.
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
fn resolve_oracle(ours: &Path) -> String {
    let chosen = match std::env::var("NODE_JS_FUZZ_NODE") {
        Ok(p) => {
            let Some(abs) = absolutize(&p) else {
                eprintln!("parity-fuzz: NODE_JS_FUZZ_NODE={p}: not found");
                std::process::exit(2);
            };
            if version_of(&abs).is_none() {
                eprintln!("parity-fuzz: NODE_JS_FUZZ_NODE={p}: not a usable node");
                std::process::exit(2);
            }
            abs
        }
        Err(_) => {
            let mut found = None;
            for p in [
                "node",
                "/opt/homebrew/bin/node",
                "/usr/local/bin/node",
                "/usr/bin/node",
            ] {
                if let Some(abs) = absolutize(p) {
                    if version_of(&abs).is_some() {
                        found = Some(abs);
                        break;
                    }
                }
            }
            match found {
                Some(p) => p,
                None => {
                    eprintln!("parity-fuzz: no reference node found; set NODE_JS_FUZZ_NODE");
                    std::process::exit(2);
                }
            }
        }
    };
    // Two ways a "node" on PATH is not the oracle this harness needs, both of
    // which make every case agree and the whole run vacuous:
    //
    //  * it IS the binary under test (a `node` shim pointing at node-js, or a
    //    `target/debug` directory on PATH). Comparing a program against itself
    //    reports 100% parity while proving nothing.
    //  * it is a version-manager shim or a different program that answers
    //    `--version` with something that is not Node's `vX.Y.Z`.
    let real = std::fs::canonicalize(&chosen).unwrap_or_else(|_| PathBuf::from(&chosen));
    if std::fs::canonicalize(ours)
        .map(|o| o == real)
        .unwrap_or(false)
    {
        eprintln!(
            "parity-fuzz: the resolved oracle IS the binary under test ({}) — \
             a run against itself compares nothing; set NODE_JS_FUZZ_NODE",
            real.display()
        );
        std::process::exit(2);
    }
    let v = version_of(&chosen).unwrap_or_default();
    if !is_node_version(&v) {
        eprintln!(
            "parity-fuzz: {} answers --version with {v:?}, which is not a Node \
             version (vX.Y.Z) — refusing to treat it as the oracle",
            real.display()
        );
        std::process::exit(2);
    }
    real.to_string_lossy().into_owned()
}

/// `vMAJOR.MINOR.PATCH`, the shape every `node --version` prints. A version
/// manager's shim, or an unrelated program that happens to be named `node`,
/// answers with something else.
fn is_node_version(v: &str) -> bool {
    let Some(rest) = v.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// An absolute path for `prog`: taken as-is when it already contains a
/// separator, otherwise looked up along `PATH`. Naming the oracle by its
/// absolute path is what makes a divergence report reproducible — `node` means
/// whatever the PATH said at that moment, which differs between shells, `sudo`,
/// and CI.
fn absolutize(prog: &str) -> Option<String> {
    let p = Path::new(prog);
    if p.components().count() > 1 {
        return p.exists().then(|| prog.to_string());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(prog))
            .find(|c| c.is_file())
            .map(|c| c.to_string_lossy().into_owned())
    })
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

/// `<absolute path> (<version>)`, for the run header and the report file, so a
/// divergence record can be attributed to the exact binary that produced it —
/// not to whichever `node` the reader's PATH happens to resolve.
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

/// A parity gap: stdout bytes differ, OR the two processes exited with
/// different STATUS CODES.
///
/// This used to compare success-ness only — `(exit == 0) != (exit == 0)` — on
/// the argument that a from-scratch interpreter may pick its own nonzero code
/// for an uncaught exception. That argument is now false in both directions.
/// It is false about node-js, which reproduces Node's codes exactly (1 for an
/// uncaught throw and an unhandled rejection, the requested code for
/// `process.exit(n)`, `n & 0xff` for `process.exitCode = n`). And it made an
/// entire observable unreachable: `process.exitCode = 3` prints NOTHING, so a
/// harness that collapses the status to a boolean cannot see it at all — which
/// is exactly how `process.exitCode` came to be stored, read back, and then
/// ignored at exit without either harness noticing. Comparing the code exactly
/// is what makes the `exit` mode below able to report anything.
fn differs(oracle: &RunOut, ours: &RunOut) -> bool {
    if oracle.exit != ours.exit {
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
    // Pin the locale and timezone rather than inheriting the developer's.
    //
    // Both were an axis this harness silently held at whatever the author's
    // shell happened to export. Reference `node` is NOT locale- or TZ-invariant
    // — `(1234.5).toLocaleString()` is `1.234,5` under `de_DE` and
    // `new Date(0).getHours()` is `9` under `Asia/Tokyo` — so a case touching
    // either would have agreed on the author's machine and diverged on a
    // colleague's, or vice versa, with nothing in the report to say why.
    // node-js reads no `LANG`/`LC_ALL`/`TZ` anywhere, so pinning costs it
    // nothing and makes every run reproducible.
    //
    // `UTC` is the honest choice rather than a convenient one: this runtime
    // hardwires `Date` to UTC, so pinning any OTHER zone would make every
    // local-time getter a permanent divergence that the report could not act
    // on. That gap is real and documented in BUGS.md; it is a missing feature,
    // not something a fuzz run should rediscover on every case.
    for (k, v) in [
        ("TZ", "UTC"),
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "en_US.UTF-8"),
    ] {
        cmd.env(k, v);
    }
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
    "0.1",
    "0.2",
    "0.5",
    "1.5",
    "2.5",
    "3.14",
    "10.0",
    "-1.5",
    "100.0",
    "0.0",
    "-0.0",
    "1e21",
    "1e-7",
    "1e-6",
    "1e100",
    "1.5e300",
    "123.456",
    "0.0001234",
    "9.999999e20",
    "1e22",
    "-0.4",
    "1.005",
    "8.575",
];
// Values that exercise NaN / Infinity / -0 propagation.
const SPECIALS: &[&str] = &["NaN", "Infinity", "-Infinity", "0/0", "1/0", "-1/0", "-0"];
/// String receivers. The last three are ASTRAL: every JS string index counts
/// UTF-16 code units, so a supplementary-plane character occupies TWO of them
/// and shifts every index past it. A pool of BMP-only strings makes that
/// impossible to observe — `'café'` is non-ASCII but still one code unit — so
/// the whole class of code-unit bugs was unreachable no matter how many cases
/// ran. These entries are what give the index arms below something to catch.
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
    "'𝒳'",
    "'ab𝒳cd'",
    "'😀🎉'",
];

/// Argument lists for `String.fromCharCode` / `fromCodePoint`. `fromCharCode`
/// truncates each argument to a uint16 (so a supplementary code point comes out
/// as a *different* BMP character) while `fromCodePoint` does not — the two
/// disagree on exactly these inputs, which is the point of testing both.
const CHAR_CODES: &[&str] = &["65", "97, 98", "0x263A", "0xD835, 0xDCB3", "0x1D4B3"];
const CODE_POINTS: &[&str] = &["65", "97, 98", "0x263A", "0x1D4B3", "0x1F600, 0x41"];

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
        "0",
        "1",
        "''",
        "'0'",
        "'1'",
        "'a'",
        "null",
        "undefined",
        "true",
        "false",
        "[]",
        "[0]",
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
/// template-literal interpolation of mixed types, and the code-unit-indexed
/// family (`length`, `charCodeAt`/`codePointAt`, `charAt`, `s[i]`, the search
/// quartet's position argument, `String.fromCharCode`/`fromCodePoint`).
fn gen_strmeth(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let s = pick(r, STRS);
    let idx = &["0", "1", "2", "-1", "-2", "3", "10"];
    let a = pick(r, idx);
    let b = pick(r, idx);
    let e = match r.below(25) {
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
        //
        // This pool stays BMP-only, and NOT to dodge a fixable bug: splitting an
        // astral character yields two UNPAIRED surrogates, and `util.inspect`
        // renders those as `'\ud835'` — a value a Rust `String` cannot hold, so
        // no amount of index work can make this arm agree (BUGS.md records the
        // boundary). It is pinned by an asserting test instead of fuzzed,
        // because an arm that can never go green is a permanently red gate.
        7 => format!(
            "{}.split('')",
            pick(r, &["'abc'", "'a'", "''", "'hi'", "'abcde'", "'Wor'"])
        ),
        8 => format!("{s}.replace('a', 'X')"),
        9 => format!("{s}.replaceAll('a', 'X')"),
        10 => format!("{s}.indexOf('o')"),
        11 => format!("{s}.at({a})"),
        12 => format!("`[${{{s}}}]-[${{{}}}]`", pick(r, INTS)),
        13 => format!("`${{{}}}${{{}}}`", pick(r, INTS), s),
        // ── Code-unit-sensitive arms ────────────────────────────────────────
        // Everything below reports or consumes a UTF-16 INDEX. On BMP input each
        // agrees with a code-point implementation by coincidence; on the astral
        // entries of STRS they diverge, which is what makes them worth emitting.
        // Each yields a number, a boolean, or a single string written straight
        // to stdout, so none of them depends on `util.inspect` escaping.
        14 => format!("{s}.length"),
        15 => format!("{s}.charCodeAt({a})"),
        16 => format!("{s}.codePointAt({a})"),
        17 => format!("{s}.charAt({a})"),
        18 => format!("{s}[{a}]"),
        19 => format!("{s}.indexOf('o', {a})"),
        20 => format!("{s}.lastIndexOf('o', {a})"),
        21 => format!("{s}.includes('o', {a})"),
        22 => format!("{s}.startsWith('a', {a})"),
        23 => format!("String.fromCharCode({})", pick(r, CHAR_CODES)),
        _ => format!("String.fromCodePoint({})", pick(r, CODE_POINTS)),
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
        "0",
        "1",
        "''",
        "'x'",
        "null",
        "undefined",
        "true",
        "false",
        "[]",
        "{}",
        "NaN",
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
        "'42'",
        "'42px'",
        "'  17  '",
        "'0x1f'",
        "'101'",
        "'3.14'",
        "'1e3'",
        "''",
        "'abc'",
        "'-5'",
        "'  '",
        "'0.5e2'",
        "'Infinity'",
        "'  12.5abc'",
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
        // `(-{a})` with a NEGATIVE `a` spells `(--3)`, which is `--` applied to
        // a literal — an invalid assignment target, so both sides raised the
        // same SyntaxError and the case compared two failures instead of two
        // values. That single template is the whole source of this mode's
        // `ref failed` / `ref silent` counters. Parenthesising the operand
        // keeps the intended probe (unary minus directly before `**`, which
        // needs the parens to parse at all) and never produces `--`.
        3 => format!("(-({a})) ** {exp}"),
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
        8 => format!(
            "Math.pow({}, {})",
            pick(r, POSINTS),
            pick(r, &["2", "3", "0"])
        ),
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
            "const f = x => x < 2 ? x : f(x - 1) + f(x - 2);".into(),
            format!("console.log(f({}));", 5 + r.below(8)),
        ],
        _ => vec![
            "let acc = 1;".into(),
            format!("let i = 1; while (i <= {n}) {{ acc *= i; i++; }}"),
            "console.log(acc);".into(),
        ],
    }
}

/// ES6 classes — fields, methods, inheritance + `super`, static members,
/// getters/setters, computed methods, `instanceof`. Every probe prints a
/// deterministic value (numbers/booleans/strings), never an object identity.
fn gen_class(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, INTS);
    let b = pick(r, INTS);
    match r.below(9) {
        0 => vec![
            "class C { constructor(x) { this.x = x; } dbl() { return this.x * 2; } }".into(),
            format!("console.log(new C({a}).dbl());"),
        ],
        1 => vec![
            "class A { constructor(n) { this.n = n; } val() { return this.n; } }".into(),
            "class B extends A { constructor(n) { super(n); this.m = n + 1; } val() { return super.val() + this.m; } }".into(),
            format!("console.log(new B({a}).val());"),
        ],
        2 => vec![
            format!("class C {{ static add(a, b) {{ return a + b; }} static k = {a}; }}"),
            format!("console.log(C.add({a}, {b}), C.k);"),
        ],
        3 => vec![
            "class Temp { constructor(c) { this._c = c; } get f() { return this._c * 2; } set f(v) { this._c = v - 1; } }".into(),
            format!("const t = new Temp({a}); const before = t.f; t.f = {b}; console.log(before, t.f);"),
        ],
        4 => vec![
            "class Base {} class Mid extends Base {} class Leaf extends Mid {}".into(),
            "const x = new Leaf();".into(),
            "console.log(x instanceof Leaf, x instanceof Mid, x instanceof Base, x instanceof Object);".into(),
        ],
        5 => vec![
            format!("class C {{ #secret = {a}; reveal() {{ return this.#secret + {b}; }} }}"),
            "const c = new C();".into(),
            "console.log(c.reveal(), JSON.stringify(c), Object.keys(c).length);".into(),
        ],
        6 => vec![
            "class Counter { constructor() { this.c = 0; } inc() { this.c++; return this; } }".into(),
            "const k = new Counter(); k.inc().inc().inc(); console.log(k.c);".into(),
        ],
        7 => vec![
            "class P { constructor(x, y) { this.x = x; this.y = y; } toString() { return `(${this.x},${this.y})`; } }".into(),
            format!("console.log(String(new P({a}, {b})), new P({a}, {b}).constructor.name);"),
        ],
        _ => vec![
            "class Shape { area() { return 0; } }".into(),
            "class Sq extends Shape { constructor(s) { super(); this.s = s; } area() { return this.s * this.s; } }".into(),
            format!("const arr = [new Sq({}), new Sq({})]; console.log(arr.map(x => x.area()));", 1 + r.below(6), 1 + r.below(6)),
        ],
    }
}

/// Generators — `function*`, `yield`, `yield*`, `.next()` sequencing,
/// generator-as-iterable in spread / `for-of` / `Array.from`. Deterministic
/// output only (no generator-object identity printing).
fn gen_generator(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let n = 2 + r.below(4);
    match r.below(7) {
        0 => vec![
            format!("function* g() {{ yield {}; yield {}; yield {}; }}", pick(r, INTS), pick(r, INTS), pick(r, INTS)),
            "console.log([...g()]);".into(),
        ],
        1 => vec![
            format!("function* g() {{ for (let i = 0; i < {n}; i++) yield i * {n}; }}"),
            "console.log(Array.from(g()));".into(),
        ],
        2 => vec![
            format!("function* g() {{ yield* [{}, {}]; yield {}; }}", pick(r, INTS), pick(r, INTS), pick(r, INTS)),
            "console.log([...g()]);".into(),
        ],
        3 => vec![
            "function* g() { let x = yield 1; let y = yield x + 1; yield y + 1; }".into(),
            format!("const it = g(); console.log(it.next().value, it.next({a}).value, it.next({b}).value);", a = 10 + r.below(5), b = 20 + r.below(5)),
        ],
        4 => vec![
            format!("function* g() {{ yield {}; yield {}; }}", pick(r, POSINTS), pick(r, POSINTS)),
            "let sum = 0; for (const v of g()) sum += v; console.log(sum);".into(),
        ],
        5 => vec![
            "function* range(n) { for (let i = 0; i < n; i++) yield i; }".into(),
            format!("console.log(Math.max(...range({})));", 1 + r.below(6)),
        ],
        _ => vec![
            "function* g() { yield 1; return 99; yield 2; }".into(),
            "const it = g(); console.log(it.next().value, it.next().value, it.next().done);".into(),
        ],
    }
}

/// Map / Set / WeakMap / WeakSet — construction from iterables, get/set/has/
/// delete/size/clear, insertion-order iteration, forEach, spread. Keys/values are
/// primitives so iteration order and output are deterministic.
fn gen_mapset(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, INTS);
    let b = pick(r, INTS);
    match r.below(8) {
        0 => vec![format!(
            "const m = new Map(); m.set('a', {a}); m.set('b', {b}); console.log(m.size, m.get('a'), m.has('b'), [...m.keys()]);"
        )],
        1 => vec![format!(
            "const s = new Set([{a}, {b}, {a}, {b}]); console.log(s.size, [...s]);"
        )],
        2 => vec![format!(
            "const m = new Map([['x', {a}], ['y', {b}]]); console.log([...m.values()], [...m.entries()]);"
        )],
        3 => vec![format!(
            "const s = new Set(); s.add({a}); s.add({b}); s.delete({a}); console.log(s.has({a}), s.size);"
        )],
        4 => vec![
            format!("const m = new Map([['k', {a}]]); let out = []; m.forEach((v, k) => out.push(k + ':' + v)); console.log(out);"),
        ],
        5 => vec![format!(
            "const s = new Set([1, 2, 3, 4]); const doubled = [...s].map(x => x * {}); console.log(doubled);", 1 + r.below(4)
        )],
        6 => vec![
            "const m = new Map(); m.set(1, 'a'); m.set(2, 'b'); m.clear(); console.log(m.size, [...m.keys()]);".into()
        ],
        _ => vec![format!(
            "const wm = new WeakMap(); const k = {{}}; wm.set(k, {a}); console.log(wm.get(k), wm.has(k), wm.has({{}}));"
        )],
    }
}

/// The ES2025 set operations (`union`/`intersection`/`difference`/
/// `symmetricDifference`/`isSubsetOf`/`isSupersetOf`/`isDisjointFrom`) against
/// both a real `Set` and a SET-LIKE operand (`{size, has, keys}`).
///
/// Each operation branches on the two sizes and walks the smaller side, and
/// which side is walked decides the RESULT ORDER and which of the operand's
/// methods runs — so the sizes are varied deliberately rather than left equal,
/// and the set-like arm is emitted as often as the plain one. The error paths
/// (a non-object operand, a NaN or negative `size`, a non-callable `has`) are
/// generated too: their messages are observable through `e.message`.
fn gen_setops(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const OPS: &[&str] = &["union", "intersection", "difference", "symmetricDifference"];
    const PREDS: &[&str] = &["isSubsetOf", "isSupersetOf", "isDisjointFrom"];
    // Deliberately different lengths so both size branches are exercised.
    const LEFT: &[&str] = &["[1,2,3]", "[3,1]", "[]", "[2]", "[1,2,3,4,5]"];
    const RIGHT: &[&str] = &["[2,3]", "[9]", "[]", "[1,2,3]", "[3,4]"];
    let l = pick(r, LEFT);
    let rt = pick(r, RIGHT);
    match r.below(7) {
        0 => vec![format!(
            "console.log([...new Set({l}).{}(new Set({rt}))]);",
            pick(r, OPS)
        )],
        1 => vec![format!(
            "console.log(new Set({l}).{}(new Set({rt})));",
            pick(r, PREDS)
        )],
        // A set-like operand: `has`/`keys` are what the spec calls, so a run
        // that only ever passes real Sets never checks that they are called.
        2 => vec![
            format!("const other = {{ size: {rt}.length, has: v => {rt}.includes(v), keys: () => {rt}[Symbol.iterator]() }};"),
            format!("console.log([...new Set({l}).{}(other)]);", pick(r, OPS)),
        ],
        3 => vec![
            format!("const other = {{ size: {rt}.length, has: v => {rt}.includes(v), keys: () => {rt}[Symbol.iterator]() }};"),
            format!("console.log(new Set({l}).{}(other));", pick(r, PREDS)),
        ],
        // The result is a plain Set even for a subclass receiver, and never
        // aliases either operand.
        4 => vec![
            format!("class S extends Set {{}}; const a = new S({l}); const b = new Set({rt});"),
            format!("const c = a.{}(b);", pick(r, OPS)),
            "console.log(c.constructor.name, c === a, c === b, c instanceof Set);".into(),
        ],
        5 => vec![
            format!("const bad = {};", pick(r, &["7", "'s'", "null", "{ size: NaN, has(){}, keys(){} }", "{ size: -1, has(){}, keys(){} }", "{ size: 1, has: 1, keys(){} }"])),
            format!("try {{ new Set({l}).{}(bad); console.log('no throw'); }} catch (e) {{ console.log(e.constructor.name, e.message); }}", pick(r, OPS)),
        ],
        // `-0`/`NaN` are SameValueZero keys: `-0` normalizes to `0` and `NaN`
        // matches itself, in the result as well as in the membership tests.
        _ => vec![format!(
            "console.log([...new Set([-0, NaN, 1]).{}(new Set([0, NaN, 2]))]);",
            pick(r, OPS)
        )],
    }
}

/// A STRING viewed as an object: its own index keys, `length`, and the
/// descriptors of both. `Object.keys`/`values`/`entries`/`assign`, spread,
/// `for-in` and `getOwnPropertyNames`/`Descriptor` are all the same underlying
/// question asked through different doors, and node-js answered them
/// inconsistently for a string PRIMITIVE (empty) while agreeing for the boxed
/// `new String(...)` — so both spellings are generated for every view.
fn gen_strkeys(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    const STRS: &[&str] = &["''", "'a'", "'ab'", "'abc'", "'a b'", "'\\u00e9x'"];
    let s = pick(r, STRS);
    let boxed = if r.below(2) == 0 {
        s.to_string()
    } else {
        format!("new String({s})")
    };
    match r.below(8) {
        0 => vec![format!("console.log(Object.keys({boxed}), Object.values({boxed}));")],
        1 => vec![format!("console.log(Object.entries({boxed}));")],
        2 => vec![format!(
            "console.log(Object.getOwnPropertyNames({boxed}));"
        )],
        3 => vec![format!(
            "console.log(Object.getOwnPropertyDescriptor({boxed}, {}), Object.getOwnPropertyDescriptor({boxed}, 'length'));",
            r.below(4)
        )],
        4 => vec![format!(
            "console.log(JSON.stringify(Object.assign({{}}, {boxed})), JSON.stringify({{ ...{s} }}));"
        )],
        5 => vec![format!(
            "const out = []; for (const k in {boxed}) out.push(k); console.log(out);"
        )],
        6 => vec![format!(
            "console.log(Object.getOwnPropertyDescriptors({boxed}));"
        )],
        // `Symbol.toStringTag` decides the brand for EVERY string conversion,
        // not only the explicit `Object.prototype.toString.call`.
        _ => vec![
            format!("const o = {}; ", pick(r, &["{ [Symbol.toStringTag]: 'T' }", "new (class { get [Symbol.toStringTag]() { return 'D'; } })()", "{}", "new Map()"])),
            "console.log(String(o), `${o}`, o + '', o.toString(), Object.prototype.toString.call(o));".into(),
        ],
    }
}

/// Prototype chain, `instanceof`, `in`, `hasOwnProperty`, `Object.getPrototypeOf`
/// / `create` / `setPrototypeOf`, `Symbol` basics + `Symbol.iterator`. All output
/// is a deterministic boolean/number/string (never a symbol identity address).
fn gen_proto(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, INTS);
    match r.below(13) {
        0 => vec![format!(
            "const o = {{ a: {a}, b: 2 }}; console.log('a' in o, 'c' in o, o.hasOwnProperty('a'), o.hasOwnProperty('toString'));"
        )],
        1 => vec![
            "const proto = { greet() { return 'hi'; } };".into(),
            "const o = Object.create(proto); o.x = 1;".into(),
            "console.log(o.greet(), o.hasOwnProperty('greet'), Object.getPrototypeOf(o) === proto);".into(),
        ],
        2 => vec![
            "function Animal(n) { this.n = n; } Animal.prototype.speak = function() { return this.n; };".into(),
            "const d = new Animal('rex'); console.log(d.speak(), d instanceof Animal, d.constructor === Animal);".into(),
        ],
        3 => vec![
            "console.log([] instanceof Array, [] instanceof Object, ({}) instanceof Object, (() => {}) instanceof Function);".into()
        ],
        4 => vec![
            "const a = Symbol('x'), b = Symbol('x');".into(),
            "console.log(typeof a, a.description, a === b, Symbol.for('k') === Symbol.for('k'));".into(),
        ],
        5 => vec![
            "const o = {};".into(),
            "o[Symbol.iterator] = function*() { yield 1; yield 2; yield 3; };".into(),
            "console.log([...o]);".into(),
        ],
        6 => vec![
            format!("const base = {{ v: {a} }}; const o = {{ __proto__: base }};"),
            "console.log(o.v, Object.getPrototypeOf(o) === base);".into(),
        ],
        7 => vec![
            "class A {} class B extends A {}".into(),
            "console.log(Object.getPrototypeOf(B.prototype) === A.prototype, new B() instanceof A);".into(),
        ],
        8 => vec![
            "const o = { get x() { return 42; } };".into(),
            "console.log(o.x, Object.getOwnPropertyDescriptor(o, 'x').get !== undefined);".into(),
        ],
        // `.constructor.name` on builtin instances and boxed primitives.
        9 => vec![format!(
            "console.log([].constructor.name, ({{}}).constructor.name, ({a}).constructor.name, 'x'.constructor.name, true.constructor.name);"
        )],
        // `.constructor.name` on Map/Set/Promise instances.
        10 => vec![
            "console.log(new Map().constructor.name, new Set().constructor.name, Promise.resolve(1).constructor.name);".into(),
        ],
        // `Ctor.name` on builtin constructors vs `undefined` on non-callable namespaces.
        11 => vec![
            "console.log(Array.name, Object.name, Number.name, String.name, Symbol.name, Promise.name, Map.name, Set.name, Error.name, TypeError.name, typeof Math.name, typeof JSON.name);".into(),
        ],
        // A runtime-thrown error reports its type through `.constructor.name`.
        _ => vec![
            "let r;".into(),
            "try { null.x; } catch (e) { r = [e instanceof TypeError, e instanceof Error, e.constructor.name]; }".into(),
            "console.log(r);".into(),
        ],
    }
}

/// Async / Promise ordering — DETERMINISTIC output only: microtask vs timer
/// ordering, `.then` chains, `Promise.all`/`race`, `async`/`await`, and the
/// sync→nextTick→microtask→timer sequence. Never prints anything time- or
/// identity-dependent; every path settles synchronously-schedulable values.
fn gen_async(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let a = pick(r, INTS);
    let b = pick(r, INTS);
    match r.below(8) {
        0 => vec![format!(
            "Promise.resolve({a}).then(v => v * 2).then(v => console.log(v));"
        )],
        1 => vec![
            "console.log('sync');".into(),
            "Promise.resolve().then(() => console.log('micro'));".into(),
            "console.log('sync2');".into(),
        ],
        2 => vec![
            format!("async function f() {{ const x = await Promise.resolve({a}); const y = await {b}; return x + y; }}"),
            "f().then(v => console.log(v));".into(),
        ],
        3 => vec![format!(
            "Promise.all([Promise.resolve({a}), Promise.resolve({b}), 7]).then(a => console.log(a));"
        )],
        4 => vec![
            "console.log(1);".into(),
            "Promise.resolve().then(() => console.log(3));".into(),
            "process.nextTick(() => console.log(2));".into(),
            "console.log('end');".into(),
        ],
        5 => vec![
            "setTimeout(() => console.log('timer'), 0);".into(),
            "Promise.resolve().then(() => console.log('promise'));".into(),
            "console.log('main');".into(),
        ],
        6 => vec![
            format!("async function f() {{ try {{ await Promise.reject(new Error('e{a}')); }} catch (e) {{ return e.message; }} }}"),
            "f().then(v => console.log(v));".into(),
        ],
        _ => vec![format!(
            "Promise.race([Promise.resolve('a'), Promise.resolve('b')]).then(v => console.log(v)); Promise.allSettled([Promise.resolve({a}), Promise.reject({b})]).then(r => console.log(r.map(x => x.status)));"
        )],
    }
}

/// BigInt — `10n` literal arithmetic (`+ - * / % **`, truncating division/sign),
/// bitwise, comparisons across BigInt/Number, the mix-`TypeError`, `typeof`,
/// formatting (`String`/`toString`/`toString(radix)`), and the `BigInt(...)`
/// constructor. Every probe prints a deterministic value or a caught error message.
fn gen_bigint(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let bigs = &[
        "0n", "1n", "2n", "3n", "7n", "10n", "-3n", "-7n", "42n", "100n", "255n", "1000n",
    ];
    let a = pick(r, bigs);
    let b = pick(r, bigs);
    let nb = pick(r, &["1", "2", "10", "-3", "0", "100"]); // a Number, for the mix path
    let e = match r.below(14) {
        0 => format!("{a} + {b}"),
        1 => format!("{a} - {b}"),
        2 => format!("{a} * {b}"),
        // Division/modulo guard against a zero divisor (a deterministic RangeError
        // otherwise, which is fine, but keep the arithmetic clean).
        3 => format!("({b} === 0n) ? 0n : {a} / {b}"),
        4 => format!("({b} === 0n) ? 0n : {a} % {b}"),
        // Parenthesize the base: JS makes `-x ** y` a SyntaxError (unary minus
        // directly before `**`), so the base is wrapped — same as the `arith` mode.
        5 => format!("({a}) ** {}", pick(r, &["0n", "1n", "2n", "3n"])),
        6 => format!("{a} < {b}, {a} > {b}, {a} <= {b}, {a} >= {b}"),
        7 => format!("{a} == {nb}, {a} === {nb}, {a} != {nb}"),
        8 => format!("typeof {a}, String({a}), {a}.toString()"),
        9 => format!("{a}.toString({})", 2 + r.below(35)),
        10 => format!("{a} & {b}, {a} | {b}, {a} ^ {b}, ~{a}"),
        // The BigInt/Number mix is a hard TypeError — verify the exact message.
        11 => format!(
            "(() => {{ try {{ return {a} + {nb}; }} catch (e) {{ return e.message; }} }})()"
        ),
        12 => format!(
            "BigInt({}), BigInt('{}'), BigInt(true)",
            pick(r, &["5", "42", "-7", "0"]),
            pick(r, &["10", "255", "0x1f"])
        ),
        _ => format!("{a} + {b} * {}", pick(r, bigs)),
    };
    vec![format!("console.log({e})")]
}

/// Regex — ONLY the Rust-`regex`-supported subset (char classes, quantifiers,
/// anchors, groups, alternation, `\d\w\s`, flags `g`/`i`/`m`), with a fixed
/// input set so output is deterministic. Never emits backreferences/lookaround
/// (node-js rejects those by design). Probes access `[0]`/`.index`/`.length`/
/// `.groups` rather than printing a raw match array (whose extra-prop inspect is a
/// documented gap).
fn gen_regex(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    // (pattern, an input that the pattern can match somewhere)
    let cases: &[(&str, &str)] = &[
        (r"\d+", "abc123def456"),
        (r"[a-z]+", "Hello World"),
        (r"\w+", "foo_bar baz"),
        (r"a|b", "xayb"),
        (r"^\w+", "start here"),
        (r"\d{2,3}", "a1b22c333"),
        (r"(ab)+", "xababy"),
        (r"\s+", "a  b   c"),
        (r"[0-9]{4}", "year 2024 end"),
        (r"[A-Z]\w*", "Foo bar Baz"),
        // Astral inputs: `.index`, `lastIndex` and a replace callback's offset
        // are UTF-16 code-unit offsets, so a supplementary character BEFORE the
        // match shifts every one of them by one. The patterns here are
        // digit/ASCII-letter classes that cannot match the astral character
        // itself, which keeps this probing the INDEX and not the separate
        // question of what `\w`/`.` mean outside the BMP.
        (r"\d+", "a𝒳b123def"),
        (r"[a-z]+", "𝒳Hello World"),
        (r"[0-9]{4}", "😀year 2024 end"),
    ];
    let (pat, input) = *pick(r, cases);
    let e = match r.below(10) {
        0 => format!("/{pat}/.test(\"{input}\")"),
        1 => format!("/{pat}/.exec(\"{input}\")[0]"),
        2 => format!("/{pat}/.exec(\"{input}\").index"),
        3 => format!("\"{input}\".match(/{pat}/g)"),
        4 => format!("\"{input}\".replace(/{pat}/g, \"#\")"),
        5 => format!("\"{input}\".replace(/{pat}/, \"#\")"),
        6 => format!("\"{input}\".split(/{pat}/)"),
        7 => format!("\"{input}\".search(/{pat}/)"),
        8 => format!("[...\"{input}\".matchAll(/{pat}/g)].map(m => m[0])"),
        _ => format!("/{pat}/i.test(\"{input}\"), /{pat}/.source, /{pat}/g.flags"),
    };
    vec![format!("console.log({e})")]
}

/// Property descriptors — the attribute triple, accessors, and every reader
/// that has to agree about which own keys exist and which of them enumerate
/// (`keys`/`values`/`entries`/spread/`assign`/`JSON`/`for-in`/
/// `getOwnPropertyNames`/`propertyIsEnumerable`). The generated object always
/// mixes a data property, a `defineProperty` data property with explicit flags,
/// and an accessor, so key ORDER is asserted as well as key membership.
fn gen_descriptor(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let enumerable = pick(r, &["true", "false"]);
    let writable = pick(r, &["true", "false"]);
    let configurable = pick(r, &["true", "false"]);
    let acc_enum = pick(r, &["true", "false"]);
    let mut setup = vec![
        "const o = {first: 1};".to_string(),
        format!(
            "Object.defineProperty(o, 'mid', {{value: 2, enumerable: {enumerable}, \
             writable: {writable}, configurable: {configurable}}});"
        ),
        format!(
            "Object.defineProperty(o, 'acc', {{get() {{ return 3; }}, enumerable: {acc_enum}, \
             configurable: true}});"
        ),
        "o.last = 4;".to_string(),
    ];
    // Sometimes mutate through the (possibly non-writable) slot first.
    if r.below(2) == 0 {
        setup.push("o.mid = 99;".to_string());
    }
    let e = match r.below(11) {
        0 => "Object.keys(o)",
        1 => "Object.values(o)",
        2 => "Object.entries(o)",
        3 => "JSON.stringify(o)",
        4 => "JSON.stringify({...o})",
        5 => "JSON.stringify(Object.assign({}, o))",
        6 => "Object.getOwnPropertyNames(o)",
        7 => "JSON.stringify(Object.getOwnPropertyDescriptor(o, 'mid'))",
        8 => "[o.propertyIsEnumerable('mid'), o.propertyIsEnumerable('acc'), o.propertyIsEnumerable('last')]",
        9 => "[Object.prototype.hasOwnProperty.call(o, 'mid'), Object.hasOwn(o, 'acc'), Object.hasOwn(o, 'nope')]",
        _ => "(() => { const ks = []; for (const k in o) ks.push(k); return ks; })()",
    };
    setup.push(format!("console.log({e});"));
    setup
}

/// Object freeze/seal/preventExtensions: the sloppy-mode write outcomes and the
/// `isFrozen`/`isSealed`/`isExtensible` predicates.
fn gen_freeze(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let op = pick(r, &["freeze", "seal", "preventExtensions"]);
    let lit = pick(r, &["{a: 1, b: 2}", "{x: 'v'}", "{}", "{n: 0, m: null}"]);
    let mutation = pick(
        r,
        &["o.a = 9;", "o.fresh = 1;", "delete o.a;", "o.b = o.b;"],
    );
    let e = match r.below(5) {
        0 => "JSON.stringify(o)",
        1 => "[Object.isFrozen(o), Object.isSealed(o), Object.isExtensible(o)]",
        2 => "Object.keys(o)",
        3 => "Object.getOwnPropertyNames(o)",
        _ => "JSON.stringify(Object.getOwnPropertyDescriptors(o))",
    };
    vec![
        format!("const o = Object.{op}({lit});"),
        mutation.to_string(),
        format!("console.log({e});"),
    ]
}

/// Builtin identity and prototype-chain reads: the `===` guards packages use to
/// type-test values they did not construct. Every probe has a fixed boolean or
/// string answer, never an address.
fn gen_identity(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let e = match r.below(12) {
        0 => "[Math === Math, JSON === JSON, Reflect === Reflect]",
        1 => "[Array.prototype === Array.prototype, Object.prototype === Object.prototype]",
        2 => "[Object.getPrototypeOf([]) === Array.prototype, Object.getPrototypeOf({}) === Object.prototype]",
        3 => "[Object.getPrototypeOf(new Map()) === Map.prototype, Object.getPrototypeOf(new Set()) === Set.prototype]",
        4 => "String(Object.getPrototypeOf(Object.create(null)))",
        5 => "(() => { class C {} return [Object.getPrototypeOf(new C()) === C.prototype, Object.getPrototypeOf(C.prototype) === Object.prototype]; })()",
        6 => "(() => { class A {} class B extends A {} return [Object.getPrototypeOf(B.prototype) === A.prototype, new B() instanceof A]; })()",
        7 => "(() => { function F() {} return [Object.getPrototypeOf(new F()) === F.prototype, F.prototype.constructor === F]; })()",
        8 => "[[] instanceof Array, [] instanceof Object, /x/ instanceof RegExp, new Date(0) instanceof Date]",
        9 => "[Object.keys(Object.prototype).length, Object.keys(Array.prototype).length]",
        10 => "(() => { const p = {i: 1}; const o = Object.create(p); o.own = 2; const ks = []; for (const k in o) ks.push(k); return [ks, Object.keys(o)]; })()",
        _ => "[Object.prototype.toString.call([]), Object.prototype.toString.call(null), Object.prototype.toString.call(new Map())]",
    };
    vec![format!("console.log({e});")]
}

/// `structuredClone` — deep copy, reference-graph preservation, cycles, and the
/// structured types (Map/Set/Date/RegExp/typed array) that are not plain
/// objects. Every probe prints a shape or a boolean, never an identity.
fn gen_clone(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let e = match r.below(9) {
        0 => "JSON.stringify(structuredClone({a: 1, b: [1, [2, 3]], c: {d: 'x'}}))",
        1 => "(() => { const s = {a: [1, 2]}; const c = structuredClone(s); c.a.push(3); return [JSON.stringify(s), JSON.stringify(c)]; })()",
        2 => "(() => { const sh = {id: 1}; const c = structuredClone({p: sh, q: sh}); return [c.p === c.q, c.p !== sh]; })()",
        3 => "(() => { const cy = {n: 'r'}; cy.self = cy; const c = structuredClone(cy); return [c.self === c, c.n]; })()",
        4 => "(() => { const c = structuredClone(new Map([['k', [1, 2]]])); return [c instanceof Map, c.size, JSON.stringify([...c])]; })()",
        5 => "(() => { const c = structuredClone(new Set([1, 2, 2])); return [c instanceof Set, c.size, JSON.stringify([...c])]; })()",
        6 => "(() => { const c = structuredClone(new Date(86400000)); return [c instanceof Date, c.toISOString()]; })()",
        7 => "(() => { const c = structuredClone(/a+b/gim); return [c instanceof RegExp, c.source, c.flags]; })()",
        _ => "(() => { const c = structuredClone(new Uint8Array([7, 8, 9])); return [c instanceof Uint8Array, c.length, c[1]]; })()",
    };
    vec![format!("console.log({e});")]
}

/// Error object shape: own-property enumerability, `cause`, `AggregateError`,
/// subclassing, and the `String(err)`/`JSON.stringify(err)` renderings. Stack
/// text is never printed — its frames are engine-specific.
fn gen_error(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let ctor = pick(
        r,
        &[
            "Error",
            "TypeError",
            "RangeError",
            "SyntaxError",
            "EvalError",
        ],
    );
    let msg = pick(r, &["boom", "", "with spaces", "sym#1"]);
    let e = match r.below(9) {
        0 => format!("JSON.stringify(Object.keys(new {ctor}('{msg}')))"),
        1 => format!("Object.getOwnPropertyNames(new {ctor}('{msg}')).sort()"),
        2 => format!("JSON.stringify(new {ctor}('{msg}'))"),
        3 => format!("[String(new {ctor}('{msg}')), new {ctor}('{msg}').name, new {ctor}('{msg}').message]"),
        4 => format!("JSON.stringify(Object.getOwnPropertyDescriptor(new {ctor}('{msg}'), 'message'))"),
        5 => format!("(() => {{ const e = new {ctor}('{msg}', {{cause: 'why'}}); return [e.cause, JSON.stringify(Object.keys(e))]; }})()"),
        6 => format!("(() => {{ const e = new {ctor}('{msg}'); e.extra = 1; return [JSON.stringify(Object.keys(e)), JSON.stringify(e)]; }})()"),
        7 => format!("(() => {{ class E extends {ctor} {{ constructor(m) {{ super(m); this.name = 'E'; }} }} const e = new E('{msg}'); return [String(e), e instanceof {ctor}, e instanceof Error, JSON.stringify(Object.keys(e))]; }})()"),
        _ => format!("(() => {{ const a = new AggregateError([new {ctor}('{msg}')], 'agg'); return [a.errors.length, a.message, JSON.stringify(Object.keys(a)), JSON.stringify(a)]; }})()"),
    };
    vec![format!("console.log({e});")]
}

// ---------------------------------------------------------------------------
// Abrupt completions (`unwind` mode)
// ---------------------------------------------------------------------------

/// One abrupt statement drawn from the set legal at this emission site,
/// optionally guarded so it fires on one iteration only. Two statements in the
/// same template share a guard on purpose: the interesting cases are the ones
/// where a `throw` and a `return`/`break` are live SIMULTANEOUSLY (a `finally`
/// that completes abruptly DISCARDS a pending exception), which a generator that
/// staggers them across iterations can never reach.
fn abrupt(r: &mut Rng, kinds: &[&str], guard: Option<&str>) -> String {
    let s = *pick(r, kinds);
    match guard {
        Some(g) => format!("if ({g}) {{ {s} }}"),
        None => s.to_string(),
    }
}

/// Wrap `body` in one loop. Every kind ADVANCES unconditionally (the `while` /
/// `do-while` forms increment at the top of the body), so a generated `continue`
/// — including one raised from a `finally` — can never spin forever.
fn loop_wrap(kind: u64, var: &str, label: Option<&str>, n: u64, body: &str) -> String {
    let lab = label.map(|l| format!("{l}: ")).unwrap_or_default();
    // The body sits inside its own BLOCK SCOPE. A jump that leaves the loop from
    // inside a `try` has to close that scope on the way out; if it does not, the
    // frame is left in a dead child env and the scope-liveness tail catches it.
    let body =
        format!("{{ let z_{var} = 'z' + {var}; if (z_{var} === '') out.push(z_{var}); {body} }}");
    match kind % 5 {
        0 => format!("{lab}for (let {var} = 0; {var} < {n}; {var}++) {{ {body} }}"),
        1 => format!("{lab}for (const {var} of [0, 1, 2, 3].slice(0, {n})) {{ {body} }}"),
        2 => format!("{lab}for (const {var} in {{ 0: 'a', 1: 'b', 2: 'c' }}) {{ {body} }}"),
        3 => format!("{{ let {var} = -1; {lab}while ({var} + 1 < {n}) {{ {var}++; {body} }} }}"),
        _ => {
            format!("{{ let {var} = -1; {lab}do {{ {var}++; {body} }} while ({var} + 1 < {n}); }}")
        }
    }
}

/// Abrupt-completion fuzzer: `break`/`continue` (labeled and unlabeled), `return`
/// and `throw` crossing `for`/`for-of`/`for-in`/`while`/`do-while`, `switch`,
/// labeled blocks, `try`, `catch` and `finally`, nested up to two loop levels —
/// and, deliberately, the same shapes with NO enclosing loop at all, where the
/// only catcher of a `break` is a `switch` or a labeled block.
///
/// Every construct appends to one `out` array and the program prints
/// `out.join(',')`, so a mis-routed jump shows up as a WRONG OR TRUNCATED
/// sequence rather than as a crash. An escaping `throw` is caught by an outer
/// handler and recorded, keeping it on stdout where the comparison can see it.
fn gen_unwind(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let n = 2 + r.below(3);
    let k = r.below(n);
    let guard = format!("String(j) === '{k}'");
    let g = Some(guard.as_str());
    // `step` is incremented BEFORE the switch, so `- 1` makes the first visit
    // select `case 0` — where the interesting `try` bodies live.
    let sw = "(step - 1) % 3";
    // Abrupt statements by what encloses the emission site.
    let in_loop: &[&str] = &["break;", "continue;", "throw new Error('E');"];
    let in_loop2: &[&str] = &[
        "break;",
        "continue;",
        "break L0;",
        "continue L0;",
        "throw new Error('E');",
    ];
    let in_fn_loop: &[&str] = &[
        "break;",
        "continue;",
        "return 'R';",
        "throw new Error('E');",
    ];
    // The loop-free templates decide UP FRONT whether they are wrapped in a
    // function, because `return` outside one is a SyntaxError under `node -e`
    // (which evaluates a Script, not a CommonJS module).
    let wrapped = r.below(2) == 0;
    // Template 9 always wraps, so `return` is unconditionally legal there.
    let fn_ret_or_throw: &[&str] = &["throw new Error('E');", "return 'R';"];
    let in_switch: &[&str] = if wrapped {
        &["break;", "throw new Error('E');", "return 'R';"]
    } else {
        &["break;", "throw new Error('E');"]
    };
    let labeled: &[&str] = if wrapped {
        &["break L0;", "throw new Error('E');", "return 'R';"]
    } else {
        &["break L0;", "throw new Error('E');"]
    };

    // Each template is `(body, needs_fn)`. `needs_fn` wraps it in an arrow so a
    // generated `return` is legal and its value is observable.
    let (body, needs_fn) = match r.below(10) {
        // A loop whose body is try/catch/finally, both slots able to fire together.
        0 => {
            let a = abrupt(r, in_loop, g);
            let b = abrupt(r, in_loop, g);
            (
                loop_wrap(
                    r.next_u64(),
                    "j",
                    None,
                    n,
                    &format!(
                        "step++; try {{ {a} out.push('t' + j); }} \
                         catch (e) {{ out.push('c' + e.message); }} \
                         finally {{ out.push('f' + j); {b} }}"
                    ),
                ),
                false,
            )
        }
        // …the same, inside a function, so `return` competes with the others.
        1 => {
            let a = abrupt(r, in_fn_loop, g);
            let b = abrupt(r, in_fn_loop, g);
            (
                loop_wrap(
                    r.next_u64(),
                    "j",
                    None,
                    n,
                    &format!(
                        "step++; try {{ {a} out.push('t' + j); }} finally {{ out.push('f' + j); {b} }}"
                    ),
                ),
                true,
            )
        }
        // A `switch` inside a loop: `break` binds to the switch, `continue` skips it.
        2 => {
            let a = abrupt(r, in_loop, g);
            let b = abrupt(r, in_loop, g);
            (
                loop_wrap(
                    r.next_u64(),
                    "j",
                    None,
                    n,
                    &format!(
                        "step++; switch ({sw}) {{ case 0: out.push('s0'); {a} \
                         case 1: out.push('s1'); break; default: out.push('sd'); {b} }} \
                         out.push('post' + j);"
                    ),
                ),
                false,
            )
        }
        // A `try` inside a `switch` inside a loop.
        3 => {
            let a = abrupt(r, in_loop, g);
            let b = abrupt(r, in_loop, g);
            (
                loop_wrap(
                    r.next_u64(),
                    "j",
                    None,
                    n,
                    &format!(
                        "step++; switch ({sw}) {{ case 0: try {{ {a} out.push('a' + j); }} \
                         finally {{ out.push('f' + j); {b} }} default: out.push('d' + j); }}"
                    ),
                ),
                false,
            )
        }
        // Two loop levels, the inner body carrying a try — labeled jumps cross both.
        4 => {
            let a = abrupt(r, in_loop2, g);
            let b = abrupt(r, in_loop2, g);
            let inner = loop_wrap(
                r.next_u64(),
                "j",
                None,
                n,
                &format!(
                    "step++; try {{ {a} out.push('t' + j); }} \
                     catch (e) {{ out.push('c' + e.message); }} \
                     finally {{ out.push('f' + j); {b} }}"
                ),
            );
            (
                loop_wrap(
                    r.next_u64(),
                    "i",
                    Some("L0"),
                    n,
                    &format!("out.push('i' + i); {inner}"),
                ),
                false,
            )
        }
        // Two loop levels with the labeled jump raised from a `switch` in the inner
        // body — the `break` target (`switch`) and the `continue` target (the outer
        // loop) are then DIFFERENT contexts.
        5 => {
            let a = abrupt(r, in_loop2, g);
            let inner = loop_wrap(
                r.next_u64(),
                "j",
                None,
                n,
                &format!(
                    "step++; switch ({sw}) {{ case 0: try {{ {a} }} \
                     finally {{ out.push('f' + j); }} default: out.push('d' + j); }}"
                ),
            );
            (
                loop_wrap(
                    r.next_u64(),
                    "i",
                    Some("L0"),
                    n,
                    &format!("out.push('i' + i); {inner}"),
                ),
                false,
            )
        }
        // NO enclosing loop: a bare `switch` whose case holds a `try`. The only
        // context that can catch the `break` is the `switch` itself.
        6 => {
            let a = abrupt(r, in_switch, None);
            let b = abrupt(r, in_switch, None);
            (
                format!(
                    "const j = 0; step++; switch ({sw}) {{ case 0: try {{ {a} out.push('a'); }} \
                     finally {{ out.push('f'); {b} }} out.push('post'); \
                     default: out.push('d'); }}"
                ),
                wrapped,
            )
        }
        // NO enclosing loop: a LABELED `switch`, exited by `break L0` from a `try`.
        7 => {
            let a = abrupt(r, labeled, None);
            (
                format!(
                    "const j = 0; step++; L0: switch ({sw}) {{ case 0: try {{ {a} out.push('a'); }} \
                     finally {{ out.push('f'); }} default: out.push('d'); }} out.push('end');"
                ),
                wrapped,
            )
        }
        // NO enclosing loop: a labeled BLOCK exited from inside a `try`.
        8 => {
            let a = abrupt(r, labeled, None);
            let b = abrupt(r, labeled, None);
            (
                format!(
                    "const j = 0; L0: {{ try {{ {a} out.push('a'); }} \
                     finally {{ out.push('f'); {b} }} out.push('post'); }} out.push('end');"
                ),
                wrapped,
            )
        }
        // A `finally` that completes abruptly while an exception is pending — the
        // case where the abrupt completion DISCARDS the throw.
        _ => {
            let a = abrupt(r, fn_ret_or_throw, None);
            let b = abrupt(r, fn_ret_or_throw, None);
            let inner = format!(
                "try {{ out.push('t'); {a} }} finally {{ out.push('f'); {b} }} out.push('post');"
            );
            let shaped = match r.below(3) {
                0 => inner,
                1 => format!("try {{ {inner} }} catch (e) {{ out.push('c' + e.message); }}"),
                _ => format!("try {{ {inner} }} finally {{ out.push('F'); }}"),
            };
            (format!("const j = 0; step++; {shaped}"), true)
        }
    };
    let body = if needs_fn {
        format!("const run = () => {{ {body} return 'end'; }}; out.push(run());")
    } else {
        body
    };
    vec![
        "const out = []; let step = 0;".into(),
        format!("try {{ {body} }} catch (e) {{ out.push('X' + e.message); }}"),
        // Scope-liveness tail. A `break`/`continue` raised as a signal out of a
        // `try` jumps straight to its target; if that jump does not close the
        // block scopes it passed through, the frame is left standing in a dead
        // child env and THIS `const` binds somewhere the closure below cannot
        // see it. Without the tail the leak is invisible — the program still
        // prints the right sequence.
        "const tail = out.length; const probe = () => tail;".into(),
        "console.log(out.join(',') + '|' + probe());".into(),
    ]
}

// ---------------------------------------------------------------------------
// Promise resolution + async ordering (`thenable` mode)
// ---------------------------------------------------------------------------

/// Promise-resolution and async-iteration ORDERING fuzzer. Each case runs one
/// async construct against a ruler of chained `.then` callbacks, so the printed
/// interleaving encodes exactly how many microtask ticks the construct costs —
/// the observable that separates a spec-faithful `PromiseResolveThenableJob` /
/// `AsyncGeneratorYield` from a shortcut that settles a tick early.
///
/// Everything is deterministic: no timers, no wall clock, no identities.
fn gen_thenable(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let ruler = 4 + r.below(6);
    // The value an `await`/`then`/`yield` is handed. Each costs a different,
    // spec-defined number of ticks.
    let val = *pick(
        r,
        &[
            "'v'",
            "Promise.resolve('v')",
            "{ then(res) { res('v'); } }",
            "{ then(res) { Promise.resolve().then(() => res('v')); } }",
            "(async () => 'v')()",
            "Promise.resolve(Promise.resolve('v'))",
        ],
    );
    let probe = match r.below(8) {
        0 => format!("(async () => {{ log('a0'); log('a1:' + await ({val})); }})();"),
        1 => format!("Promise.resolve().then(() => ({val})).then(v => log('t:' + v));"),
        2 => format!("new Promise(res => res({val})).then(v => log('n:' + v));"),
        3 => format!("Promise.resolve({val}).then(v => log('r:' + v));"),
        4 => format!(
            "(async function* () {{ yield {val}; yield 'w'; }})()[Symbol.asyncIterator]; \
             (async () => {{ for await (const v of (async function* () {{ yield {val}; yield 'w'; }})()) log('g:' + v); log('gdone'); }})();"
        ),
        5 => format!(
            "(async () => {{ for await (const v of [{val}, 'w']) log('l:' + v); log('ldone'); }})();"
        ),
        6 => format!(
            "const it = (async function* () {{ yield {val}; }})(); \
             it.next().then(s => log('s:' + s.value + ',' + s.done)); \
             it.next().then(s => log('e:' + s.value + ',' + s.done));"
        ),
        _ => format!(
            "(async () => {{ for await (const v of (function* () {{ yield {val}; yield 'w'; }})()) log('y:' + v); log('ydone'); }})();"
        ),
    };
    vec![
        "const log = s => console.log(s);".into(),
        probe,
        format!(
            "let p = Promise.resolve(); for (let i = 1; i <= {ruler}; i++) {{ const n = i; p = p.then(() => log('p' + n)); }}"
        ),
    ]
}

/// Process EXIT status: `process.exitCode`, `process.exit`, and the `exit` /
/// `beforeExit` events.
///
/// This mode exists because the two harnesses shared a blind spot with an exact
/// shape: neither ever emitted `process.exitCode`, and both collapsed the exit
/// status to zero-vs-nonzero. `process.exitCode = 3` produces no stdout, so it
/// was invisible on the only other axis either one compares — and the property
/// was in fact stored, readable, and then ignored at exit. The generator half
/// only works alongside the exact-code comparison in `differs`.
///
/// Everything printed is fixed text; the varying part is the STATUS, which is
/// now a compared observable.
fn gen_exit(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let code = *pick(r, &["0", "1", "3", "7", "42", "255", "300", "-1"]);
    let strcode = *pick(r, &["'3'", "'0x10'", "'  '", "'1e3'"]);
    match r.below(10) {
        0 => vec![format!("process.exitCode = {code};")],
        1 => vec![format!("process.exitCode = {strcode};")],
        2 => vec![format!("console.log('a'); process.exit({code});")],
        3 => vec![format!(
            "process.exitCode = {code}; process.exit(); console.log('unreachable');"
        )],
        4 => vec![
            "process.on('exit', c => console.log('exit', c));".into(),
            format!("process.exitCode = {code};"),
        ],
        5 => vec![
            "process.on('exit', c => console.log('exit', c));".into(),
            format!("process.exit({code});"),
        ],
        6 => vec![format!(
            "process.on('exit', () => {{ process.exitCode = {code}; }});"
        )],
        7 => vec![
            "process.on('beforeExit', c => console.log('beforeExit', c));".into(),
            "process.on('exit', c => console.log('exit', c));".into(),
            format!("process.exitCode = {code};"),
        ],
        8 => vec![
            "process.once('e', () => console.log('once'));".into(),
            "process.on('e', () => console.log('on'));".into(),
            "process.emit('e'); process.emit('e');".into(),
            "console.log('left', process.listeners('e').length);".into(),
        ],
        _ => vec![
            "process.on('exit', c => console.log('exit', c));".into(),
            format!("setTimeout(() => {{ process.exitCode = {code}; }}, 0);"),
        ],
    }
}

/// Raw STDIO writes: `process.stdout.write` / `process.stderr.write` with every
/// chunk form, including bytes that are not valid UTF-8 and chunks with no
/// trailing newline.
///
/// No generator emitted `process.stdout.write` at all, so the whole
/// ToString-the-chunk path went unmeasured — a `Buffer` chunk printed the 15
/// bytes of `[object Object]`, and `write('4142','hex')` printed the literal.
/// The comparison side was always ready for this: the fuzzer compares raw
/// stdout BYTES, and run.sh uses `cmp`.
fn gen_stdio(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let enc = *pick(r, &["'utf8'", "'hex'", "'base64'", "'latin1'", "'utf16le'"]);
    let text = *pick(r, &["'4142'", "'QUJD'", "'ab'", "'\\u00e9'", "'0f10'"]);
    match r.below(9) {
        0 => vec![format!("process.stdout.write({text}, {enc});")],
        1 => vec!["process.stdout.write('a'); process.stdout.write('b');".into()],
        2 => vec!["process.stdout.write(Buffer.from([0xff, 0xfe, 0x41, 0x00, 0x7f]));".into()],
        3 => vec![format!(
            "process.stdout.write(new Uint8Array([{}, {}, 65]));",
            r.below(256),
            r.below(256)
        )],
        4 => vec![
            "console.log('before');".into(),
            "process.stdout.write(Buffer.from('mid'));".into(),
            "console.log('after');".into(),
        ],
        5 => vec!["process.stdout.end('tail');".into()],
        6 => vec![format!(
            "try {{ process.stdout.write({}); }} catch (e) {{ console.log(e.constructor.name, e.message); }}",
            pick(r, &["[65, 66]", "65", "undefined", "null", "{}"])
        )],
        7 => vec![format!(
            "process.stdout.write(Buffer.from({text}, {enc}));"
        )],
        _ => vec!["process.stdout.write(''); console.log('empty-ok');".into()],
    }
}

/// Entry-point observables: top-level `this`, `globalThis` identity, and the
/// module bindings a `-e` script sees.
///
/// Round 4 established that these differ BY ENTRY POINT, and used that to argue
/// the two harnesses should stay on different ones. That is right, and it also
/// left the values unmeasured at BOTH: every generator avoided them, so
/// `typeof this` was `undefined` at all three entry points, and `globalThis`
/// minted a fresh object per read, undetected. What is emitted here is only
/// what is entry-point INVARIANT (identity relations, `typeof`) or specific to
/// `-e`, which is the entry point this harness drives.
fn gen_entry(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let probe = *pick(
        r,
        &[
            "typeof this",
            "this === globalThis",
            "this === module.exports",
            "globalThis === globalThis",
            "globalThis === global",
            "typeof globalThis",
            "typeof global",
            "typeof require",
            "typeof module",
            "typeof exports",
            "typeof arguments",
            "typeof __filename",
            "typeof __dirname",
            "module.id",
            "require.main === module",
            "process.argv.length",
            "Array.isArray(process.argv)",
            "typeof process.exitCode",
        ],
    );
    match r.below(4) {
        0 => vec![format!("console.log({probe});")],
        1 => vec![
            "globalThis.__probe = 7;".into(),
            "console.log(globalThis.__probe, typeof globalThis.__probe);".into(),
        ],
        2 => vec![
            "this.__x = 5;".into(),
            "console.log(this.__x, module.exports.__x);".into(),
        ],
        _ => vec![
            "const g = globalThis;".into(),
            format!("g.__y = 1; console.log(globalThis.__y, {probe});"),
        ],
    }
}

/// The locale/`toLocale*` surface, plus the case and normalization methods that
/// sit next to it.
///
/// Deliberately EXCLUDED from the determinism invariant's reach: node-js reads
/// no `LANG`, `LC_ALL` or `TZ` anywhere (verified by grep over `src/`) and
/// hardwires `Date` to UTC and the number formats to en-US, so these outputs
/// are fixed on every machine. Reference `node` is not locale-invariant, which
/// is why the harness now pins the environment before comparing — see the env
/// block in `run_prog`. Without both halves this mode would be a machine-
/// dependent test, which is the failure it is meant to prevent.
fn gen_locale(seed: u64) -> Vec<String> {
    let r = &mut Rng::new(seed);
    let n = *pick(
        r,
        &[
            "0",
            "1234.5",
            "1234567.891",
            "-9876.5",
            "1e21",
            "0.000001",
            "123456789012345",
            "-0",
        ],
    );
    let s = *pick(
        r,
        &["'abc'", "'Straße'", "'ÄÖÜ'", "'I'", "'i'", "'\\u00e9'"],
    );
    let ms = *pick(r, &["0", "86400000", "1700000000123", "-86400000", "NaN"]);
    let e = match r.below(12) {
        0 => format!("({n}).toLocaleString()"),
        1 => format!("({n}).toLocaleString('en-US')"),
        2 => format!("BigInt(Math.trunc({n} || 0)).toLocaleString()"),
        3 => format!("[{n}, {s}, null, undefined].toLocaleString()"),
        4 => format!("({s}).toLocaleUpperCase()"),
        5 => format!("({s}).toLocaleLowerCase()"),
        6 => format!("({s}).toLocaleString()"),
        7 => format!("new Date({ms}).toLocaleDateString()"),
        8 => format!("new Date({ms}).toLocaleTimeString()"),
        9 => format!("new Date({ms}).toLocaleString()"),
        10 => format!("({s}).normalize('NFC') === ({s})"),
        _ => "({}).toLocaleString()".to_string(),
    };
    vec![format!("console.log({e});")]
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
    Class,
    Generator,
    MapSet,
    SetOps,
    StrKeys,
    Proto,
    Async,
    Bigint,
    Regex,
    Descriptor,
    Freeze,
    Identity,
    Clone,
    ErrorShape,
    Unwind,
    Thenable,
    Exit,
    Stdio,
    Entry,
    Locale,
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
    Mode::Class,
    Mode::Generator,
    Mode::MapSet,
    Mode::SetOps,
    Mode::StrKeys,
    Mode::Proto,
    Mode::Async,
    Mode::Bigint,
    Mode::Regex,
    Mode::Descriptor,
    Mode::Freeze,
    Mode::Identity,
    Mode::Clone,
    Mode::ErrorShape,
    Mode::Unwind,
    Mode::Thenable,
    Mode::Exit,
    Mode::Stdio,
    Mode::Entry,
    Mode::Locale,
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
        Mode::Class => gen_class(seed),
        Mode::Generator => gen_generator(seed),
        Mode::MapSet => gen_mapset(seed),
        Mode::SetOps => gen_setops(seed),
        Mode::StrKeys => gen_strkeys(seed),
        Mode::Proto => gen_proto(seed),
        Mode::Async => gen_async(seed),
        Mode::Bigint => gen_bigint(seed),
        Mode::Regex => gen_regex(seed),
        Mode::Descriptor => gen_descriptor(seed),
        Mode::Freeze => gen_freeze(seed),
        Mode::Identity => gen_identity(seed),
        Mode::Clone => gen_clone(seed),
        Mode::ErrorShape => gen_error(seed),
        Mode::Unwind => gen_unwind(seed),
        Mode::Thenable => gen_thenable(seed),
        Mode::Exit => gen_exit(seed),
        Mode::Stdio => gen_stdio(seed),
        Mode::Entry => gen_entry(seed),
        Mode::Locale => gen_locale(seed),
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
        Mode::Class => "class",
        Mode::Generator => "generator",
        Mode::MapSet => "mapset",
        Mode::SetOps => "setops",
        Mode::StrKeys => "strkeys",
        Mode::Proto => "proto",
        Mode::Async => "async",
        Mode::Bigint => "bigint",
        Mode::Regex => "regex",
        Mode::Descriptor => "descriptor",
        Mode::Freeze => "freeze",
        Mode::Identity => "identity",
        Mode::Clone => "clone",
        Mode::ErrorShape => "error",
        Mode::Unwind => "unwind",
        Mode::Thenable => "thenable",
        Mode::Exit => "exit",
        Mode::Stdio => "stdio",
        Mode::Entry => "entry",
        Mode::Locale => "locale",
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
    Mode::Class,
    Mode::Generator,
    Mode::MapSet,
    Mode::SetOps,
    Mode::StrKeys,
    Mode::Proto,
    Mode::Async,
    Mode::Bigint,
    Mode::Regex,
    Mode::Descriptor,
    Mode::Freeze,
    Mode::Identity,
    Mode::Clone,
    Mode::ErrorShape,
    Mode::Unwind,
    Mode::Thenable,
    Mode::Exit,
    Mode::Stdio,
    Mode::Entry,
    Mode::Locale,
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

/// The `--help` mode list is DERIVED from [`ALL_MODES`], never typed out. The
/// literal it replaced listed 13 of the then-27 real modes: every mode added
/// after it was written (`class` through `thenable`) worked, was accepted by
/// `mode_from_name`, and was undiscoverable. A hand-maintained copy of a list
/// that lives in the source goes stale the first time the source changes.
fn mode_list() -> String {
    let names: Vec<&str> = ALL_MODES.iter().copied().map(mode_name).collect();
    let mut out = String::new();
    for chunk in names.chunks(8) {
        if !out.is_empty() {
            out.push_str("\n                  ");
        }
        out.push_str(&chunk.join(", "));
    }
    out
}

fn print_help() {
    eprintln!(
        "parity-fuzz — differential node/node-js parity fuzzer\n\
         \n\
         --count N        number of cases (default 2000)\n\
         --seed N         base seed; case i uses seed+i (default 1)\n\
         --mode M         one of ({} modes; `mixed` rotates every other one):\n\
         \x20                 {}\n\
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
         (HARD ERROR if set but unusable). Every run prints the oracle it used.\n\
         Both children run with TZ=UTC LANG=LC_ALL=en_US.UTF-8, pinned rather\n\
         than inherited, so a run reproduces on any machine.",
        ALL_MODES.len(),
        mode_list()
    );
}

fn main() {
    let args = parse_args();
    let bin = ours_bin();
    let oracle = resolve_oracle(&bin);
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
    // Cases where the REFERENCE itself failed or printed nothing. Such a case
    // compares two failures, so an "agreement" on it is worth nothing — a mode
    // whose zero-divergence score is built on them is measuring nothing at all.
    let oracle_failed = AtomicU64::new(0);
    let oracle_silent = AtomicU64::new(0);
    // The only condition under which a case really observed NOTHING: the
    // reference printed nothing AND exited 0, so neither compared channel
    // carried a value. `ref failed` and `ref silent` each catch one channel
    // being empty, which stopped being the same thing once the exit code became
    // a compared observable — the whole `exit` mode consists of programs that
    // print nothing and exit non-zero on purpose, and it would otherwise report
    // 100% "compared two failures" while comparing exactly what it means to.
    let oracle_inert = AtomicU64::new(0);
    // An ORACLE timeout is not a comparison: the case is skipped, so a run full
    // of them would otherwise report a clean zero. Counted on its own line.
    let oracle_timeouts = AtomicU64::new(0);
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
                if o.timed_out {
                    oracle_timeouts.fetch_add(1, Ordering::Relaxed);
                }
                if !o.timed_out && o.exit != 0 {
                    oracle_failed.fetch_add(1, Ordering::Relaxed);
                }
                if !o.timed_out && o.stdout.is_empty() {
                    oracle_silent.fetch_add(1, Ordering::Relaxed);
                }
                if !o.timed_out && o.stdout.is_empty() && o.exit == 0 {
                    oracle_inert.fetch_add(1, Ordering::Relaxed);
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
    let oracle_failed = oracle_failed.load(Ordering::Relaxed);
    let oracle_silent = oracle_silent.load(Ordering::Relaxed);
    let oracle_inert = oracle_inert.load(Ordering::Relaxed);
    println!(
        "\nfuzzed {checked} cases in {:.1}s ({:.0}/s)\n\
         oracle      : {}\n\
         divergences : {} ({} known / {} new)\n\
         timeouts    : {}\n\
         ref timeout : {} (reference timed out — SKIPPED, never counted as agreement)\n\
         ref failed  : {} (reference exited non-zero — a COMPARED value since\n\
         \x20             exit codes are matched exactly; only `ref inert` means nothing was seen)\n\
         ref silent  : {} (reference printed nothing on stdout)\n\
         ref inert   : {} (reference printed nothing AND exited 0 — no value was\n\
         \x20             observed on either compared channel)",
        elapsed.as_secs_f64(),
        checked as f64 / elapsed.as_secs_f64().max(0.001),
        oracle,
        divergences.len(),
        known,
        new_records.len(),
        timeouts,
        oracle_timeouts.load(Ordering::Relaxed),
        oracle_failed,
        oracle_silent,
        oracle_inert,
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
