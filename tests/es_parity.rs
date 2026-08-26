//! Focused parity tests for the ECMAScript features fixed/added in the
//! change-by-copy + numeric/prototype sweep. Each expected value was captured
//! from system `node v26.5.0`; the tests drive the built `node` binary
//! (`CARGO_BIN_EXE_node`) as a subprocess so `console.log` output is exact and
//! no Node install is needed in CI. These pin behavior that the `examples/*.js`
//! snapshot does not already cover.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a spawned `node` gets before it is treated as hung.
///
/// Every program here is a fraction of a second's work — the whole file runs in
/// about two seconds — so a child still alive after this is not slow, it is
/// stuck. Without a bound one hung child blocks `output()` forever and takes
/// the CI job with it: the run of 2026-08-25 sat for six hours until GitHub
/// cancelled it, which reports as "cancelled" and names no test. With the
/// bound, the test that hung is the test that fails.
const CHILD_BUDGET: Duration = Duration::from_secs(60);

/// [`run_bounded`] in the shape the older call sites read: a `std::process::
/// Output`, so `out.status.success()` and `out.stdout` keep working.
fn run_bounded_out(path: &std::path::Path) -> std::process::Output {
    let (ok, stdout, stderr) = run_bounded(path);
    std::process::Output {
        status: exit_status(ok),
        stdout: stdout.into_bytes(),
        stderr: stderr.into_bytes(),
    }
}

/// An `ExitStatus` standing for success or failure. `ExitStatus` cannot be
/// constructed portably, so this runs the platform's own true/false.
fn exit_status(ok: bool) -> std::process::ExitStatus {
    Command::new(if ok { "true" } else { "false" })
        .status()
        .expect("run true/false")
}

/// The exit status, stdout and stderr of the built `node` running `path`, or a
/// panic naming the program if it outlives [`CHILD_BUDGET`].
///
/// Output goes to files rather than pipes: a pipe that fills while nothing
/// reads it is its own deadlock, and polling `try_wait` is what makes the
/// deadline enforceable.
fn run_bounded(path: &std::path::Path) -> (bool, String, String) {
    let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("node");
    let dir = std::env::temp_dir();
    let (op, ep) = (
        dir.join(format!("{stem}.{}.out", std::process::id())),
        dir.join(format!("{stem}.{}.err", std::process::id())),
    );
    let (of, ef) = (
        std::fs::File::create(&op).expect("stdout file"),
        std::fs::File::create(&ep).expect("stderr file"),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(of))
        .stderr(Stdio::from(ef))
        .spawn()
        .expect("spawn node binary");
    let deadline = Instant::now() + CHILD_BUDGET;
    let status = loop {
        match child.try_wait().expect("wait for node binary") {
            Some(st) => break st,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let out = std::fs::read_to_string(&op).unwrap_or_default();
                let err = std::fs::read_to_string(&ep).unwrap_or_default();
                let _ = std::fs::remove_file(&op);
                let _ = std::fs::remove_file(&ep);
                panic!(
                    "node did not exit within {}s — killed.\n--- stdout ---\n{out}\n--- stderr ---\n{err}",
                    CHILD_BUDGET.as_secs()
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let out = std::fs::read_to_string(&op).unwrap_or_default();
    let err = std::fs::read_to_string(&ep).unwrap_or_default();
    let _ = std::fs::remove_file(&op);
    let _ = std::fs::remove_file(&ep);
    (status.success(), out, err)
}

/// Run `src` through the built `node` binary, returning trimmed stdout. Panics
/// with stderr on a non-zero exit so a thrown error surfaces in the failure.
fn run(src: &str) -> String {
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(src.as_bytes()).expect("write source");
    let (ok, stdout, stderr) = run_bounded(f.path());
    if !ok {
        panic!("program failed:\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}");
    }
    stdout.trim_end().to_string()
}

// ── Array.prototype.flat(depth) ──────────────────────────────────────────────

#[test]
fn array_flat_honors_depth() {
    // Default depth 1, explicit finite depth, full-flatten via Infinity, and
    // depth 0 (a shallow copy that flattens nothing).
    let src = r#"
        console.log(JSON.stringify([1,[2,[3]]].flat()));
        console.log(JSON.stringify([1,[2,[3]]].flat(1)));
        console.log(JSON.stringify([1,[2,[3]]].flat(2)));
        console.log(JSON.stringify([1,[2,[3]]].flat(Infinity)));
        console.log(JSON.stringify([1,[2,[3]]].flat(0)));
        console.log(JSON.stringify([1,[2,[3,[4]]]].flat(Infinity)));
        console.log(JSON.stringify([1,[2,[3]]].flat(NaN)));
    "#;
    assert_eq!(
        run(src),
        "[1,2,[3]]\n[1,2,[3]]\n[1,2,3]\n[1,2,3]\n[1,[2,[3]]]\n[1,2,3,4]\n[1,[2,[3]]]"
    );
}

// ── Number.prototype.toString(radix) with a fractional receiver ──────────────

#[test]
fn number_tostring_radix_fraction() {
    let src = r#"
        console.log((3.5).toString(2));
        console.log((255.5).toString(16));
        console.log((-3.5).toString(2));
        console.log((0.1).toString(2));
        console.log((255).toString(16));   // integer: unaffected
        console.log((0).toString(2));
        console.log((1.5).toString(10));   // radix 10 stays fmt_number
    "#;
    assert_eq!(
        run(src),
        "11.1\nff.8\n-11.1\n0.0001100110011001100110011001100110011001100110011001101\nff\n0\n1.5"
    );
}

// ── Object.create(null) instanceof Object ────────────────────────────────────

#[test]
fn null_proto_object_is_not_object_instance() {
    let src = r#"
        console.log(Object.create(null) instanceof Object);   // false
        console.log(({}) instanceof Object);                  // true
        console.log(Object.create({}) instanceof Object);     // true
        const o = {}; Object.setPrototypeOf(o, null);
        console.log(o instanceof Object);                     // false
        const p = Object.create(null); Object.setPrototypeOf(p, Object.prototype);
        console.log(p instanceof Object);                     // true again
    "#;
    assert_eq!(run(src), "false\ntrue\ntrue\nfalse\ntrue");
}

// ── ES2023 change-by-copy array methods ──────────────────────────────────────

#[test]
fn array_to_sorted_is_a_copy() {
    let src = r#"
        const a = [3,1,2];
        console.log(JSON.stringify(a.toSorted()));
        console.log(JSON.stringify(a));                       // original intact
        console.log(JSON.stringify([3,1,2,10].toSorted((x,y)=>x-y)));
    "#;
    assert_eq!(run(src), "[1,2,3]\n[3,1,2]\n[1,2,3,10]");
}

#[test]
fn array_to_reversed_is_a_copy() {
    let src = r#"
        const b = [1,2,3];
        console.log(JSON.stringify(b.toReversed()));
        console.log(JSON.stringify(b));
    "#;
    assert_eq!(run(src), "[3,2,1]\n[1,2,3]");
}

#[test]
fn array_to_spliced_is_a_copy() {
    let src = r#"
        const c = [1,2,3,4];
        console.log(JSON.stringify(c.toSpliced(1,2,9,9,9)));
        console.log(JSON.stringify(c));
        console.log(JSON.stringify([1,2,3,4].toSpliced(-2)));   // negative start
        console.log(JSON.stringify([1,2,3,4].toSpliced(1)));    // delete to end
    "#;
    assert_eq!(run(src), "[1,9,9,9,4]\n[1,2,3,4]\n[1,2]\n[1]");
}

#[test]
fn array_with_copies_and_rangechecks() {
    let src = r#"
        const d = [1,2,3];
        console.log(JSON.stringify(d.with(1,99)));
        console.log(JSON.stringify(d.with(-1,99)));            // negative index
        console.log(JSON.stringify(d));                        // original intact
        try { [1,2,3].with(5,0); } catch (e) {
            console.log(e.constructor.name, JSON.stringify(e.message));
        }
        try { [1,2,3].with(-5,0); } catch (e) {
            console.log(e.constructor.name, JSON.stringify(e.message));
        }
    "#;
    assert_eq!(
        run(src),
        "[1,99,3]\n[1,2,99]\n[1,2,3]\nRangeError \"Invalid index : 5\"\nRangeError \"Invalid index : -5\""
    );
}

// ── Labeled statements bind to their loop target (BUGS.md was stale) ──────────

#[test]
fn labeled_continue_and_break_target_the_loop() {
    let src = r#"
        const c = [];
        outer: for (let i=0;i<3;i++) {
            for (let j=0;j<3;j++) { if (j===1) continue outer; c.push(i+":"+j); }
        }
        console.log(c.join(","));
        const b = [];
        loop: for (let i=0;i<5;i++) { if (i===2) break loop; b.push(i); }
        console.log(b.join(","));
    "#;
    assert_eq!(run(src), "0:0,1:0,2:0\n0,1");
}

// ── Object.groupBy / Map.groupBy (ES2024) ────────────────────────────────────

#[test]
fn object_group_by_null_proto_object() {
    let src = r#"
        const items = [{t:'a',n:1},{t:'b',n:2},{t:'a',n:3}];
        const g = Object.groupBy(items, x => x.t);
        console.log(JSON.stringify(g));
        console.log(Object.getPrototypeOf(g) === null);      // null-prototype
        // Second callback arg is the index.
        const g2 = Object.groupBy([10,20,30,40], (v,i) => i%2===0 ? 'even' : 'odd');
        console.log(JSON.stringify(g2));
    "#;
    assert_eq!(
        run(src),
        "{\"a\":[{\"t\":\"a\",\"n\":1},{\"t\":\"a\",\"n\":3}],\"b\":[{\"t\":\"b\",\"n\":2}]}\n\
         true\n\
         {\"even\":[10,30],\"odd\":[20,40]}"
    );
}

#[test]
fn map_group_by_returns_a_map() {
    let src = r#"
        const items = [{t:'a',n:1},{t:'b',n:2},{t:'a',n:3}];
        const m = Map.groupBy(items, x => x.t);
        console.log(m instanceof Map, m.size);
        console.log(JSON.stringify(m.get('a')), JSON.stringify(m.get('b')));
        // Map keys use SameValueZero, so object keys stay distinct.
        const k1 = {}, k2 = {};
        const m2 = Map.groupBy([1,2,3], (v,i) => i < 2 ? k1 : k2);
        console.log(JSON.stringify(m2.get(k1)), JSON.stringify(m2.get(k2)), m2.size);
    "#;
    assert_eq!(run(src), "true 2\n[{\"t\":\"a\",\"n\":1},{\"t\":\"a\",\"n\":3}] [{\"t\":\"b\",\"n\":2}]\n[1,2] [3] 2");
}

// ── Promise.withResolvers (ES2024) ───────────────────────────────────────────

#[test]
fn promise_with_resolvers_resolve_and_reject() {
    let src = r#"
        const { promise, resolve, reject } = Promise.withResolvers();
        console.log(promise instanceof Promise, typeof resolve, typeof reject);
        promise.then(v => console.log('resolved', v));
        resolve(42);
        const r = Promise.withResolvers();
        r.promise.catch(e => console.log('caught', e));
        r.reject('boom');
    "#;
    assert_eq!(run(src), "true function function\nresolved 42\ncaught boom");
}

// ── Map/Set/Promise structural instanceof ────────────────────────────────────

#[test]
fn builtin_container_instanceof() {
    let src = r#"
        console.log(new Map() instanceof Map, new Map() instanceof WeakMap, new Map() instanceof Object);
        console.log(new WeakMap() instanceof WeakMap, new WeakMap() instanceof Map);
        console.log(new Set() instanceof Set, new WeakSet() instanceof WeakSet, new WeakSet() instanceof Set);
        console.log(new Promise(()=>{}) instanceof Promise);
    "#;
    assert_eq!(
        run(src),
        "true false true\ntrue false\ntrue true false\ntrue"
    );
}

// ── Number.prototype.toLocaleString (default locale, grouped) ─────────────────

#[test]
fn number_to_locale_string_default() {
    let src = r#"
        console.log((12345.678).toLocaleString());   // 12,345.678
        console.log((1234567).toLocaleString());      // 1,234,567
        console.log((1234.5678).toLocaleString());    // rounds to 3 frac digits
        console.log((1234.9999).toLocaleString());     // rounds up to 1,235
        console.log((-9876.5).toLocaleString());
        console.log((0).toLocaleString(), (100).toLocaleString(), (1000).toLocaleString());
        console.log((-0).toLocaleString());            // keeps the sign
        console.log((NaN).toLocaleString(), (Infinity).toLocaleString(), (-Infinity).toLocaleString());
        console.log((123456789012345).toLocaleString());
    "#;
    assert_eq!(
        run(src),
        "12,345.678\n1,234,567\n1,234.568\n1,235\n-9,876.5\n0 100 1,000\n-0\nNaN \u{221e} -\u{221e}\n123,456,789,012,345"
    );
}

// ── Successful match array inspects with index/input/groups own props ─────────

#[test]
fn regex_match_array_inspect_own_props() {
    let src = r#"
        console.log('foobar'.match(/bar/));
        console.log('date 2024-01-02'.match(/(\d{4})-(\d{2})/));
        console.log('aXbXc'.match(/X/g));   // global: plain array, no own props
    "#;
    assert_eq!(
        run(src),
        "[ 'bar', index: 3, input: 'foobar', groups: undefined ]\n\
         [\n  '2024-01',\n  '2024',\n  '01',\n  index: 5,\n  input: 'date 2024-01-02',\n  groups: undefined\n]\n\
         [ 'X', 'X' ]"
    );
}

// ── Null-prototype object inspects with the [Object: null prototype] tag ──────

#[test]
fn null_proto_object_inspect_tag() {
    let src = r#"
        console.log(Object.create(null));
        const p = Object.create(null); p.x = 1; console.log(p);
    "#;
    assert_eq!(
        run(src),
        "[Object: null prototype] {}\n[Object: null prototype] { x: 1 }"
    );
}

// ── Regex backreferences + lookaround work (fancy-regex; BUGS.md was stale) ───

#[test]
fn regex_backrefs_and_lookaround() {
    let src = r#"
        console.log(/(\w)\1/.test("aa"), /(\w)\1/.test("ab"));   // backref
        console.log(/(?<=foo)bar/.test("foobar"));               // lookbehind
        console.log("foobar".replace(/(?<=foo)bar/, "X"));
        console.log(/\d+(?= dollars)/.exec("100 dollars")[0]);   // lookahead
        console.log(/(?<y>\d)\k<y>/.test("55"));                 // named backref
    "#;
    assert_eq!(run(src), "true false\ntrue\nfooX\n100\ntrue");
}

// ── Integer-key property ordering (OrdinaryOwnPropertyKeys) ───────────────────
// Array-index keys enumerate in ascending numeric order BEFORE insertion-ordered
// string keys, consistently across keys/values/entries, for-in, spread, and
// JSON.stringify — matching V8/Node.

#[test]
fn integer_key_ordering_all_enumeration_paths() {
    let src = r#"
        const o = {b:1, "2":2, a:3, "1":4, "10":5, c:6, "0":7};
        console.log(JSON.stringify(Object.keys(o)));
        console.log(JSON.stringify(Object.values(o)));
        console.log(JSON.stringify(o));                     // stringify order
        let a=[]; for (const k in o) a.push(k); console.log(a.join(","));  // for-in
        console.log(JSON.stringify({...o}));                // spread
        console.log(JSON.stringify(Object.entries(o)));
    "#;
    assert_eq!(
        run(src),
        "[\"0\",\"1\",\"2\",\"10\",\"b\",\"a\",\"c\"]\n\
         [7,4,2,5,1,3,6]\n\
         {\"0\":7,\"1\":4,\"2\":2,\"10\":5,\"b\":1,\"a\":3,\"c\":6}\n\
         0,1,2,10,b,a,c\n\
         {\"0\":7,\"1\":4,\"2\":2,\"10\":5,\"b\":1,\"a\":3,\"c\":6}\n\
         [[\"0\",7],[\"1\",4],[\"2\",2],[\"10\",5],[\"b\",1],[\"a\",3],[\"c\",6]]"
    );
}

#[test]
fn integer_key_ordering_dynamic_and_boundaries() {
    let src = r#"
        // A dynamically added array-index key re-places into ascending order.
        const d = {}; d.z=1; d["5"]=2; d.a=3; d["2"]=4;
        console.log(JSON.stringify(Object.keys(d)));
        // 2^32-1 (4294967295) is NOT an array index (stays a string key);
        // 2^32-2 (4294967294) IS.
        const b = {}; b["4294967295"]=1; b["4294967294"]=2; b.x=3; b["0"]=4;
        console.log(JSON.stringify(Object.keys(b)));
        // Leading-zero / non-canonical numeric strings are plain string keys.
        console.log(JSON.stringify(Object.keys({"01":1, "1":2, "0":3})));
        // JSON.parse result and Object.assign target both re-order.
        console.log(JSON.stringify(Object.keys(JSON.parse('{"b":1,"2":2,"a":3,"1":4}'))));
        console.log(JSON.stringify(Object.keys(Object.assign({z:1}, {"3":2}, {"1":3}))));
    "#;
    assert_eq!(
        run(src),
        "[\"2\",\"5\",\"z\",\"a\"]\n\
         [\"0\",\"4294967294\",\"4294967295\",\"x\"]\n\
         [\"0\",\"1\",\"01\"]\n\
         [\"1\",\"2\",\"b\",\"a\"]\n\
         [\"1\",\"3\",\"z\"]"
    );
}

// ── console.log object multiline breakLength wrapping ─────────────────────────
// A single-line object wider than Node's 80-char breakLength wraps one property
// per line, exactly as arrays already do (including the constructor/tag prefix).

#[test]
fn object_inspect_wraps_past_break_length() {
    // Short object stays on one line; a wide one wraps.
    assert_eq!(run("console.log({a:1,b:2,c:3});"), "{ a: 1, b: 2, c: 3 }");
    assert_eq!(
        run("console.log({aaaaaaaaaa:1,bbbbbbbbbb:2,cccccccccc:3,dddddddddd:4,eeeeeeeeee:5});"),
        "{\n  aaaaaaaaaa: 1,\n  bbbbbbbbbb: 2,\n  cccccccccc: 3,\n  dddddddddd: 4,\n  eeeeeeeeee: 5\n}"
    );
}

#[test]
fn object_inspect_wraps_class_instance_and_nesting() {
    // A class instance folds its constructor tag into the break calculation, so
    // `Point { … }` wraps when the single-line form would exceed 80 columns.
    let point = r#"
        class Point { constructor(){ this.xcoord=1; this.ycoord=2; this.zcoord=3;
            this.label="origin"; this.color="red"; } }
        console.log(new Point());
    "#;
    assert_eq!(
        run(point),
        "Point {\n  xcoord: 1,\n  ycoord: 2,\n  zcoord: 3,\n  label: 'origin',\n  color: 'red'\n}"
    );
    // The inner object fits on one line; only the outer breaks.
    assert_eq!(
        run("console.log({short:1,nested:{deep:{a:1,b:2,c:3,d:4,e:5,f:6,g:7,h:8,i:9}}});"),
        "{\n  short: 1,\n  nested: { deep: { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9 } }\n}"
    );
}

// ── FinalizationRegistry (no-GC approximation; callbacks never fire) ──────────
// The heap holds every value strongly, so cleanup callbacks never run (a
// spec-permitted behavior). The constructor and register/unregister type checks
// and the unregister bookkeeping are exact.

#[test]
fn finalization_registry_contract() {
    let src = r#"
        const reg = new FinalizationRegistry(v => v);
        const obj = {}, tok = {};
        console.log(typeof reg, reg instanceof FinalizationRegistry,
                    typeof reg.register, typeof reg.unregister);
        console.log(reg.register(obj, "held", tok));   // undefined
        console.log(reg.unregister(tok), reg.unregister(tok), reg.unregister({}));
        console.log(FinalizationRegistry.name);
    "#;
    assert_eq!(
        run(src),
        "object true function function\n\
         undefined\n\
         true false false\n\
         FinalizationRegistry"
    );
}

#[test]
fn finalization_registry_type_errors() {
    let src = r#"
        const errs = [];
        const grab = f => { try { f(); } catch (e) { errs.push(e.constructor.name); } };
        grab(() => new FinalizationRegistry(123));          // callback not callable
        grab(() => new FinalizationRegistry());             // missing callback
        const reg = new FinalizationRegistry(() => {});
        grab(() => reg.register(42, "h"));                  // target not an object
        grab(() => reg.register({}, {}, 42));               // token not an object
        const o = {};
        grab(() => reg.register(o, o));                     // target === heldValue
        grab(() => reg.unregister(42));                     // token not an object
        console.log(errs.join(","));
    "#;
    assert_eq!(
        run(src),
        "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError"
    );
}

// ── `-x ** y` is a SyntaxError, `x++ ** y` is not ────────────────────────────

/// Run `src` expecting a non-zero exit, returning trimmed stderr.
fn run_failing(src: &str) -> String {
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(src.as_bytes()).expect("write source");
    let (ok, stdout, stderr) = run_bounded(f.path());
    assert!(!ok, "expected a failure, got stdout:\n{stdout}");
    stderr.trim_end().to_string()
}

#[test]
fn unary_directly_before_exponentiation_is_a_syntax_error() {
    // `ExponentiationExpression : UpdateExpression ** …` — every UnaryExpression
    // form is rejected. Verified against node v26.7.0, which reports the same
    // message for each of these.
    for src in [
        "let x = 2, y = 3, o = { p: 1 }, a = [];\nconsole.log(-x ** y);",
        "let x = 2, y = 3;\nconsole.log(+x ** y);",
        "let x = 2, y = 3;\nconsole.log(~x ** y);",
        "let x = 2, y = 3;\nconsole.log(!x ** y);",
        "let x = 2, y = 3;\nconsole.log(typeof x ** y);",
        "let x = 2, y = 3;\nconsole.log(void x ** y);",
        "let x = 2, y = 3, o = { p: 1 };\nconsole.log(delete o.p ** y);",
        "let x = 2n, y = 3n;\nconsole.log(-x ** y);",
        // Inside a computed member and an array literal, and across a newline
        // (`**` cannot start a statement, so ASI does not rescue it).
        "let x = 2, y = 3, a = [];\nconsole.log(a[-x ** y]);",
        "let x = 2, y = 3;\nconsole.log([-x ** y]);",
        "let x = 2, y = 3;\nconsole.log(-x\n** y);",
        "async function f(x, y) { return await x ** y; }\nf(2, 3);",
    ] {
        let err = run_failing(src);
        assert!(
            err.contains(
                "SyntaxError: Unary operator used immediately before exponentiation \
                 expression. Parenthesis must be used to disambiguate operator precedence"
            ),
            "unexpected error for {src:?}:\n{err}"
        );
    }
}

#[test]
fn parenthesized_and_update_expressions_still_exponentiate() {
    // The two legal disambiguations, an UpdateExpression base (which the grammar
    // DOES allow left of `**`), right-associativity, and a unary on the RIGHT.
    let src = r#"
        let x = 2, y = 3;
        console.log((-x) ** y);
        console.log(-(x ** y));
        console.log(x ** -y);
        console.log(2 ** 3 ** 2);
        console.log((typeof x) ** y);
        console.log((-2n) ** 3n);
        let a = 2; console.log(a++ ** y);
        let b = 2; console.log(++b ** y);
    "#;
    assert_eq!(run(src), "-8\n-8\n0.125\n512\nNaN\n-8n\n8\n27");
}

// ── String search methods honor their position argument ──────────────────────

#[test]
fn string_search_methods_honor_the_position_argument() {
    // `indexOf`/`lastIndexOf`/`includes`/`startsWith`/`endsWith` all ignored
    // their 2nd argument, which made body-parser's parameterCount loop until it
    // hit the limit and reject every urlencoded body. Values from node v26.7.0.
    let src = r#"
        const s = "abcabc";
        const p = (...xs) => console.log(xs.join(","));
        p(s.indexOf("b"), s.indexOf("b",0), s.indexOf("b",2), s.indexOf("b",5), s.indexOf("b",-3), s.indexOf("b",NaN), s.indexOf("b",100));
        p(s.indexOf(""), s.indexOf("",3), s.indexOf("",100), s.indexOf("",-1));
        p(s.lastIndexOf("b"), s.lastIndexOf("b",0), s.lastIndexOf("b",3), s.lastIndexOf("b",4), s.lastIndexOf("b",-1), s.lastIndexOf("b",NaN), s.lastIndexOf("b",100));
        p(s.lastIndexOf(""), s.lastIndexOf("",2), s.lastIndexOf("",100));
        p(s.includes("b"), s.includes("b",2), s.includes("b",5), s.includes("b",-1));
        p(s.startsWith("a"), s.startsWith("a",3), s.startsWith("b",1), s.startsWith("a",-1), s.startsWith("a",100));
        p(s.endsWith("c"), s.endsWith("c",3), s.endsWith("a",1), s.endsWith("c",0), s.endsWith("c",100));
        const u = "héllo—世界";
        p(u.indexOf("世"), u.indexOf("世",5), u.indexOf("l",2), u.lastIndexOf("l"), u.lastIndexOf("l",3));
        p(s.indexOf("abcabcabc"), s.lastIndexOf("abcabcabc"), s.includes("abcabcabc"));
        p(s.indexOf("b",2.9), s.lastIndexOf("b",1.9), s.startsWith("b",1.9));
    "#;
    assert_eq!(
        run(src),
        "1,1,4,-1,1,1,-1\n\
         0,3,6,0\n\
         4,-1,1,4,-1,4,4\n\
         6,2,6\n\
         true,true,false,true\n\
         true,true,true,true,false\n\
         true,true,true,false,true\n\
         6,6,2,3,3\n\
         -1,-1,false\n\
         4,1,true"
    );
}

// ── Object.prototype.toString brand tags ─────────────────────────────────────

#[test]
fn object_prototype_tostring_brands_builtin_exotics() {
    // Everything reported `[object Object]`, which breaks the duck-typing that
    // packages use on values they did not construct. A Buffer brands as
    // Uint8Array because in Node it IS a Uint8Array subclass.
    let src = r#"
        const T = (x) => Object.prototype.toString.call(x);
        console.log([
          T(new Uint8Array(1)), T(new Float64Array(1)), T(new Int16Array(1)),
          T(new ArrayBuffer(1)), T(Buffer.from("a")), T(new Map()), T(new Set()),
          T(new WeakMap()), T(new WeakSet()), T(new Date(0)), T(Promise.resolve()),
          T(Symbol("s")), T(1n), T(new Error("x")), T(new TypeError("x")),
          T(/a/), T([1]), T({}), T(function () {}), T(null), T(undefined),
          T(1), T("s"), T(true),
        ].join("\n"));
    "#;
    assert_eq!(
        run(src),
        "[object Uint8Array]\n[object Float64Array]\n[object Int16Array]\n\
         [object ArrayBuffer]\n[object Uint8Array]\n[object Map]\n[object Set]\n\
         [object WeakMap]\n[object WeakSet]\n[object Date]\n[object Promise]\n\
         [object Symbol]\n[object BigInt]\n[object Error]\n[object Error]\n\
         [object RegExp]\n[object Array]\n[object Object]\n[object Function]\n\
         [object Null]\n[object Undefined]\n[object Number]\n[object String]\n\
         [object Boolean]"
    );
}

// ── builtin namespaces enumerate the members node-js implements ──────────────

#[test]
fn builtin_namespaces_enumerate_their_members() {
    // safer-buffer rebuilds `Buffer` with `for (key in Buffer) Safer[key] =
    // Buffer[key]`; with an unenumerable namespace that produced an empty object
    // and `Buffer.isBuffer is not a function` deep inside iconv-lite.
    let src = r#"
        const buffer = require("buffer");
        const B = buffer.Buffer;
        const Safer = {};
        for (const key in B) { if (B.hasOwnProperty(key)) Safer[key] = B[key]; }
        console.log(typeof Safer.isBuffer, typeof Safer.from, typeof Safer.alloc, typeof Safer.concat);
        console.log(Safer.isBuffer(B.from("x")), Safer.from("hi").toString());
        console.log(Object.keys(B).includes("isBuffer"), Object.keys(B).length === Object.keys(B).length);
        const mod = [];
        for (const key in buffer) mod.push(key);
        console.log(mod.includes("Buffer"), mod.includes("atob"));
        console.log(Object.keys({}).length, Object.keys([1, 2]).join(","));
    "#;
    assert_eq!(
        run(src),
        "function function function function\n\
         true hi\n\
         true true\n\
         true true\n\
         0 0,1"
    );
}

// ── AsyncResource ────────────────────────────────────────────────────────────

#[test]
fn async_resource_runs_and_binds() {
    // raw-body and on-finished both do `new AsyncResource(name)` then
    // `res.runInAsyncScope.bind(res, fn, null)`; without the class the express
    // body-parse path threw "AsyncResource is not a constructor".
    let src = r#"
        const ah = require("async_hooks");
        const { AsyncResource } = ah;
        const r = new AsyncResource("X");
        console.log(typeof r.runInAsyncScope, typeof r.emitDestroy, typeof r.bind);
        console.log(r.runInAsyncScope(function (a, b) { return [this.tag, a, b].join(","); }, { tag: "T" }, 1, 2));
        console.log(AsyncResource.bind(function (x) { return "s:" + x + ":" + this.k; }, "n", { k: 9 })(5));
        console.log(r.bind(function (x) { return "i:" + x; })(7));
        console.log(r.emitDestroy() === r);
        function wrap(fn) {
          const res = new ah.AsyncResource(fn.name || "bound-anonymous-fn");
          if (!res || !res.runInAsyncScope) return fn;
          return res.runInAsyncScope.bind(res, fn, null);
        }
        console.log(wrap(function named(a) { return "wrapped " + a; })("zz"));
    "#;
    assert_eq!(
        run(src),
        "function function function\nT,1,2\ns:5:9\ni:7\ntrue\nwrapped zz"
    );
}

// ── abrupt completions out of `try`/`finally`/`switch` ──────────────────────

#[test]
fn finally_abrupt_completion_replaces_a_pending_throw() {
    // ECMA-262 14.15.3: when the finalizer completes abruptly, ITS completion
    // wins — the exception the try/catch block left pending is DISCARDED, not
    // rethrown. Verified against node v26.7.0.
    let src = r#"
        function b() { try { throw new Error('boom'); } finally { return 2; } }
        console.log(b());
        function c() { try { return 1; } finally { return 3; } }
        console.log(c());
        function e() { try { try { throw new Error('x'); } finally { return 'inner'; } }
                       catch (err) { return 'caught ' + err.message; } }
        console.log(e());
        const o = [];
        for (let i = 0; i < 4; i++) { try { throw new Error('e' + i); } finally { o.push('f' + i); continue; } }
        console.log(o.join(','));
        function g() { try { throw new Error('a'); } catch (x) { throw new Error('b'); } finally { return 'y'; } }
        console.log(g());
    "#;
    assert_eq!(run(src), "2\n3\ninner\nf0,f1,f2,f3\ny");
}

#[test]
fn break_out_of_a_try_inside_a_switch_with_no_enclosing_loop() {
    // The `break` leaves the try's own chunk as a signal. Its target (the
    // `switch`, or a labeled block/switch) catches `break` but NOT `continue`,
    // so the two targets have to be resolved independently — resolving them
    // together made the whole program halt silently here.
    let src = r#"
        let o = [];
        switch (2) { case 2: try { o.push('a'); break; } finally { o.push('f'); } o.push('never'); }
        console.log(o.join(','));
        o = [];
        switch (2) { case 2: try { o.push('a'); break; } catch (e) {} o.push('never'); }
        console.log(o.join(','));
        o = [];
        L: switch (1) { case 1: try { break L; } finally { o.push('fin'); } }
        console.log(o.join(','));
        o = [];
        L2: { try { break L2; } finally { o.push('blk'); } o.push('never'); }
        console.log(o.join(','));
        o = [];
        for (let i = 0; i < 3; i++) { L3: switch (i) { case 1: for (const x of [1, 2, 3]) { if (x === 2) break L3; o.push('x' + x); } o.push('nope'); default: o.push('d' + i); } }
        console.log(o.join(','));
    "#;
    assert_eq!(run(src), "a,f\na\nfin\nblk\nd0,x1,d2");
}

// ── coroutine body scope isolation ───────────────────────────────────────────

#[test]
fn async_and_generator_bodies_do_not_share_top_level_bindings() {
    // A coroutine runs with a swapped-in context holding only ITS frame, so a
    // frame-COUNT test for "module scope" declared every top-level `let`/`var`
    // in an async/generator body as a GLOBAL — two interleaved activations then
    // wrote into one array.
    let src = r#"
        async function f() { let o = []; o.push('f1'); await null; o.push('f2'); return o.join(','); }
        async function g() { let o = []; o.push('g1'); await null; o.push('g2'); return o.join(','); }
        f().then(r => console.log('A', r));
        g().then(r => console.log('B', r));
        function* p() { var v = []; v.push('p1'); yield 0; v.push('p2'); return v.join(','); }
        const i1 = p(), i2 = p();
        i1.next(); i2.next();
        console.log('C', i1.next().value, i2.next().value);
    "#;
    assert_eq!(run(src), "C p1,p2 p1,p2\nA f1,f2\nB g1,g2");
}

// ── Promise resolution: thenable assimilation ────────────────────────────────

#[test]
fn resolving_with_a_thenable_adopts_it_instead_of_fulfilling_with_it() {
    // ECMA-262 27.2.1.3.2: an object with a callable `then` is adopted through
    // `NewPromiseResolveThenableJob`. Fulfilling WITH the object handed every
    // consumer the thenable itself.
    let src = r#"
        const th = { then(res) { res('TH'); } };
        new Promise(r => r(th)).then(v => console.log('N', v));
        Promise.resolve().then(() => th).then(v => console.log('T', v));
        Promise.resolve(th).then(v => console.log('R', v));
        (async () => console.log('A', await th))();
        (async function* () { yield Promise.resolve('P'); yield th; })()
          .next().then(s => console.log('G', s.value, s.done));
    "#;
    assert_eq!(run(src), "N TH\nR TH\nA TH\nG P false\nT TH");
}

#[test]
fn async_generator_queues_overlapping_next_requests() {
    // `[[AsyncGeneratorQueue]]`: a second `.next()` issued before the first
    // settles must WAIT, so the two results arrive in request order.
    let src = r#"
        const it = (async function* () { yield Promise.resolve('v'); })();
        it.next().then(s => console.log('s:' + s.value + ',' + s.done));
        it.next().then(s => console.log('e:' + s.value + ',' + s.done));
    "#;
    assert_eq!(run(src), "s:v,false\ne:undefined,true");
}

// ── unhandled promise rejections ─────────────────────────────────────────────

#[test]
fn an_unhandled_rejection_reaches_a_process_listener() {
    // Node's default is `--unhandled-rejections=throw`; a registered
    // `process.on('unhandledRejection')` listener takes it instead. The fatal
    // path is covered by `unhandled_rejection_is_fatal` below.
    let src = r#"
        process.on('unhandledRejection', (r) => console.log('caught', r.message));
        Promise.reject(new Error('z'));
        Promise.reject(new Error('q')).catch(e => console.log('handled', e.message));
        (async () => { try { await Promise.reject(new Error('aw')); } catch (e) { console.log('await', e.message); } })();
    "#;
    assert_eq!(run(src), "handled q\nawait aw\ncaught z");
}

#[test]
fn unhandled_rejection_is_fatal() {
    // No listener and no `.catch`: the process must FAIL, not exit 0 having
    // silently swallowed the error.
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(b"console.log('before'); Promise.reject(new Error('boom'));")
        .expect("write source");
    let (ok, stdout, stderr) = run_bounded(f.path());
    assert!(!ok, "an unhandled rejection must not exit 0");
    assert_eq!(stdout.trim_end(), "before");
    assert!(
        stderr.contains("boom"),
        "stderr should name the rejection: {stderr}"
    );
}

// ── ToPrimitive (7.1.1): `valueOf` / `Symbol.toPrimitive` really run ─────────

#[test]
fn to_primitive_consults_value_of_and_symbol_to_primitive() {
    // Arithmetic, relational, `==`, `ToPropertyKey` and `Array.prototype.join`
    // all convert an object through ToPrimitive. node-js used to read the raw
    // `str_of` instead, so a user `valueOf` was never invoked: `o + 1` was
    // `"[object Object]1"` and `+o` was `NaN`. A `Date` overrides the DEFAULT
    // hint to `"string"` (21.4.4.45), which is why `d + 1` concatenates while
    // `d - 0` is arithmetic.
    let src = r#"
        const o = { valueOf() { return 7; }, toString() { return 'x'; } };
        const s = { toString() { return 'S'; } };
        const v = { valueOf() { return 3; } };
        console.log([
          o + 1, 1 + o, o * 2, o - 1, +o, Number(o), o | 0,
          o == 7, o < 8, `${o}`, String(o), [o] + '', [o].join('-'),
          s + 1, `${s}`, v + 1, `${v}`,
        ].join('|'));
        const k = {}; k[o] = 1; console.log(Object.keys(k).join(','));
        const d = new Date(0);
        console.log([typeof (d + 1), d - 0, +d, d * 1].join('|'));
        const p = { [Symbol.toPrimitive](h) { return h === 'number' ? 42 : 'str:' + h; } };
        console.log([+p, `${p}`, p + '', p * 2].join('|'));
        try { Object.create(null) + 1; } catch (e) { console.log(e.constructor.name + ': ' + e.message); }
        console.log(typeof Object.create(null).toString);
    "#;
    assert_eq!(
        run(src),
        "8|8|14|6|7|7|7|true|true|x|x|x|x|S1|S|4|[object Object]\n\
         x\n\
         string|0|0|0\n\
         42|str:string|str:default|84\n\
         TypeError: Cannot convert object to primitive value\n\
         undefined"
    );
}

// ── Symbol.toStringTag / well-known symbols / symbol-keyed properties ────────

#[test]
fn to_string_tag_and_symbol_keyed_properties() {
    // `Symbol.toStringTag` overrides the builtin brand (20.1.3.6 steps 16-17),
    // generator/async functions carry their own, and `Math`/`JSON`/`Reflect`
    // brand by name rather than as callables. Symbol-keyed own properties are
    // invisible to `Object.keys`/`JSON.stringify` but ARE copied by spread and
    // `Object.assign` (7.3.25) and listed by `Reflect.ownKeys` (7.3.23).
    let src = r#"
        const T = (x) => Object.prototype.toString.call(x);
        console.log([
          T({ [Symbol.toStringTag]: 'Zed' }),
          T(new (class { get [Symbol.toStringTag]() { return 'Cee'; } })()),
          T(function* () {}), T((function* () {})()), T(async function () {}),
          T(async function* () {}), T((async function* () {})()),
          T(Math), T(JSON), T(Reflect),
          T(new WeakRef({})), T(new FinalizationRegistry(() => {})),
          T(new TextEncoder()), T(new TextDecoder()),
          T(new URL('http://a/')), T(new URLSearchParams('a=1')),
          T(new (require('events'))()), T(require('crypto').createHash('sha256')),
          String(new Map()), String(new Set()),
        ].join('\n'));
        console.log(String(Symbol.iterator), String(Symbol.toStringTag), typeof Symbol.toPrimitive);
        console.log(Symbol.for('Symbol.iterator') === Symbol.iterator);
        const s = Symbol('k');
        const src = { a: 1, [s]: 2, [Symbol.iterator]: 3 };
        console.log(Object.keys(src).join(','), JSON.stringify(src));
        console.log(Object.getOwnPropertySymbols(src).map(String).join(','));
        console.log(Reflect.ownKeys(src).map(String).join(','));
        console.log({ ...src }[s], Object.assign({}, src)[s]);
        const hid = {}; Object.defineProperty(hid, s, { value: 1, enumerable: false });
        console.log(Object.getOwnPropertySymbols(hid).length, { ...hid }[s]);
    "#;
    assert_eq!(
        run(src),
        "[object Zed]\n[object Cee]\n[object GeneratorFunction]\n[object Generator]\n\
         [object AsyncFunction]\n[object AsyncGeneratorFunction]\n[object AsyncGenerator]\n\
         [object Math]\n[object JSON]\n[object Reflect]\n\
         [object WeakRef]\n[object FinalizationRegistry]\n\
         [object TextEncoder]\n[object TextDecoder]\n\
         [object URL]\n[object URLSearchParams]\n\
         [object Object]\n[object Object]\n\
         [object Map]\n[object Set]\n\
         Symbol(Symbol.iterator) Symbol(Symbol.toStringTag) symbol\n\
         false\n\
         a {\"a\":1}\n\
         Symbol(k),Symbol(Symbol.iterator)\n\
         a,Symbol(k),Symbol(Symbol.iterator)\n\
         2 2\n\
         1 undefined"
    );
}

// ── util.inspect: symbol keys, the `[Tag]` prefix, and quote selection ───────

#[test]
fn inspect_renders_symbol_keys_tags_and_picks_its_quote() {
    // A symbol-keyed own enumerable property prints as `Symbol(desc): value`;
    // an INHERITED `Symbol.toStringTag` prints as a `Ctor [Tag] ` prefix (an
    // own enumerable one does not, since it is already listed as a property).
    // The constructor prefix now also covers `function F(){}` instances, and
    // `strEscape` picks `'`, `"` or a backtick so the contents need the least
    // escaping.
    let src = r#"
        const u = require('util');
        const s = Symbol('k');
        const base = { [Symbol.toStringTag]: 'Base' };
        function F() { this.y = 2; }
        class C { constructor() { this.x = 1; } }
        const o = Object.create(base); o.z = 3;
        console.log([
          u.inspect({ [s]: 1 }),
          u.inspect({ a: 1, [s]: 2 }),
          u.inspect({ [Symbol.toStringTag]: 'Zed', a: 1 }),
          u.inspect(o),
          u.inspect(Object.create(base)),
          u.inspect(new F()),
          u.inspect(new C()),
          u.inspect(Object.create({})),
        ].join('\n'));
        console.log([
          u.inspect("plain"), u.inspect("has'single"), u.inspect('has"double'),
          u.inspect("has'both\"kinds"), u.inspect("a'b\"c`d"),
          u.inspect("tab\there"), u.inspect("\u0000nul"), u.inspect("\u000Bvt"),
          u.inspect({ a: "it's" }),
        ].join('\n'));
    "#;
    assert_eq!(
        run(src),
        "{ Symbol(k): 1 }\n\
         { a: 1, Symbol(k): 2 }\n\
         { a: 1, Symbol(Symbol.toStringTag): 'Zed' }\n\
         Object [Base] { z: 3 }\n\
         Object [Base] {}\n\
         F { y: 2 }\n\
         C { x: 1 }\n\
         {}\n\
         'plain'\n\
         \"has'single\"\n\
         'has\"double'\n\
         `has'both\"kinds`\n\
         'a\\'b\"c`d'\n\
         'tab\\there'\n\
         '\\x00nul'\n\
         '\\x0Bvt'\n\
         { a: \"it's\" }"
    );
}

// ── Date instance methods are reachable past `Object.prototype` ─────────────

#[test]
fn date_value_of_returns_the_time_value() {
    // `valueOf`/`toString` exist on `Object.prototype` too, and node-js routed a
    // Date to that generic implementation — `d.valueOf()` returned the Date
    // itself, so `+d` and `d - 0` were NaN.
    let src = r#"
        const d = new Date(86400000);
        console.log([
          d.valueOf(), d.getTime(), typeof d.toString(), d.toISOString(),
          JSON.stringify({ d }), new Date(0) - new Date(-1000),
        ].join('|'));
    "#;
    assert_eq!(
        run(src),
        "86400000|86400000|string|1970-01-02T00:00:00.000Z|{\"d\":\"1970-01-02T00:00:00.000Z\"}|1000"
    );
}

// ── `arguments` is lexical inside an arrow ──────────────────────────────────

#[test]
fn an_arrow_sees_the_enclosing_functions_arguments() {
    // 10.2.11 creates the `arguments` binding only for a non-arrow function.
    // node-js bound a fresh empty one in EVERY call frame, so an arrow saw zero
    // arguments. A nested non-arrow still gets its own.
    let src = r#"
        function f() {
          const spread = () => [...arguments].join('-');
          const len = () => arguments.length;
          const forOf = () => { const o = []; for (const x of arguments) o.push(x); return o.join('-'); };
          const nestedArrow = () => (() => arguments.length)();
          const ownFn = function () { return arguments.length; };
          return [spread(), len(), forOf(), nestedArrow(), ownFn(9, 9)].join('|');
        }
        console.log(f(1, 2, 3));
        function g() { return Array.prototype.slice.call(arguments).join('-') + '/' + arguments.length; }
        console.log(g('a', 'b'));
    "#;
    assert_eq!(run(src), "1-2-3|3|1-2-3|3|2\na-b/2");
}

// ── an array's / a function's non-index own properties ──────────────────────

#[test]
fn an_arrays_non_index_own_properties_are_real_own_properties() {
    // A node-js array keeps its non-index own keys (`arr.foo`, `arr[sym]`, a
    // match result's `index`/`input`) in the fn-prop side table, and every
    // reflection path read only the property map — so `Object.keys`, `entries`,
    // `getOwnPropertyNames`, `Reflect.ownKeys`, spread, `for-in`, `in` and
    // `delete` all behaved as if the property did not exist.
    let src = r#"
        const s = Symbol('k');
        const a = [1, 2];
        a.foo = 'bar';
        a[s] = 5;
        console.log([
          Object.keys(a).join(','),
          Object.getOwnPropertyNames(a).join(','),
          Reflect.ownKeys(a).map(String).join(','),
          Object.getOwnPropertySymbols(a).map(String).join(','),
          JSON.stringify(Object.entries(a)),
          JSON.stringify({ ...a }),
          'foo' in a, s in a, 'nope' in a,
          JSON.stringify(a),
        ].join('|'));
        const seen = []; for (const k in a) seen.push(k);
        console.log(seen.join(','));
        console.log(delete a[s], delete a.foo, Object.keys(a).join(','), Object.getOwnPropertySymbols(a).length);
    "#;
    assert_eq!(
        run(src),
        "0,1,foo|0,1,length,foo|0,1,length,foo,Symbol(k)|Symbol(k)\
         |[[\"0\",1],[\"1\",2],[\"foo\",\"bar\"]]|{\"0\":1,\"1\":2,\"foo\":\"bar\"}\
         |true|true|false|[1,2]\n\
         0,1,foo\n\
         true true 0,1 0"
    );
}

#[test]
fn an_arrays_symbol_keys_inspect_after_the_elements() {
    // `console.log([1, 2])` with an own symbol key prints it as a trailing
    // `Symbol(desc): value`, the way an object receiver already did.
    let src = r#"
        const a = [1, 2];
        a[Symbol('k')] = 5;
        console.log(a);
        const empty = [];
        empty[Symbol('e')] = 1;
        console.log(empty);
    "#;
    assert_eq!(run(src), "[ 1, 2, Symbol(k): 5 ]\n[ Symbol(e): 1 ]");
}

#[test]
fn deleting_a_symbol_key_does_not_delete_the_string_of_the_same_name() {
    // `delete o[k]` keyed through `String(k)` rather than ToPropertyKey, so
    // `delete o[Symbol('k')]` removed a property literally named "Symbol(k)"
    // and left the symbol-keyed one in place.
    let src = r#"
        const s = Symbol('k');
        const o = { a: 1 };
        o[s] = 2;
        o['Symbol(k)'] = 'decoy';
        delete o[s];
        console.log(Object.getOwnPropertySymbols(o).length, o['Symbol(k)'], JSON.stringify(o));
    "#;
    assert_eq!(run(src), "0 decoy {\"a\":1,\"Symbol(k)\":\"decoy\"}");
}

#[test]
fn a_functions_own_properties_enumerate_and_inspect() {
    // A function's own properties live in the same side table, and its exotic
    // `length`/`name`/`prototype` are non-enumerable — so `Object.keys(f)` is
    // exactly what a script assigned while `getOwnPropertyNames(f)` reports the
    // exotics first, and `util.inspect` appends the rest in braces.
    let src = r#"
        const s = Symbol('k');
        function f(a, b) {}
        f.z = 1; f[s] = 7;
        console.log(f);
        console.log([
          Object.keys(f).join(','),
          Object.getOwnPropertyNames(f).join(','),
          Object.getOwnPropertySymbols(f).map(String).join(','),
          f.name, f.length,
        ].join('|'));
        class C { m(){} static sm(){} static x = 1; }
        console.log(C, Object.keys(C).join(','), Object.getOwnPropertyNames(C).join(','));
    "#;
    assert_eq!(
        run(src),
        "[Function: f] { z: 1, Symbol(k): 7 }\n\
         z|length,name,prototype,z|Symbol(k)|f|2\n\
         [class C] { x: 1 } x length,name,prototype,sm,x"
    );
}

#[test]
fn only_a_constructor_owns_a_prototype_property() {
    // MakeConstructor (10.2.5) runs for an ordinary function definition and for
    // every generator. An arrow, a method definition and an async function are
    // not constructors and own no `prototype` — node-js materialised one for any
    // function on first read.
    let src = r#"
        const o = { m(){}, *g(){}, async a(){}, async *ag(){} };
        class C { m(){} }
        const names = x => Object.getOwnPropertyNames(x).join(',');
        console.log([names(o.m), names(o.g), names(o.a), names(o.ag)].join('|'));
        console.log([names(() => {}), names(function(){}), names(function*(){}), names(async function(){})].join('|'));
        console.log([typeof C.prototype.m.prototype, typeof o.m.prototype, typeof o.g.prototype].join('|'));
    "#;
    assert_eq!(
        run(src),
        "length,name|length,name,prototype|length,name|length,name,prototype\n\
         length,name|length,name,prototype|length,name,prototype|length,name\n\
         undefined|undefined|object"
    );
}

#[test]
fn a_class_body_installs_methods_before_static_fields() {
    // ClassDefinitionEvaluation installs the methods while evaluating the class
    // body and runs the static-field initializers afterwards, so the own-key
    // order is methods-then-fields no matter how the source is written.
    let src = r#"
        class D { static a = 1; static m(){} static b = 2; }
        console.log(Object.getOwnPropertyNames(D).join(','));
        const E = class { static m(){} static a = 1; };
        console.log(Object.getOwnPropertyNames(E).join(','));
    "#;
    assert_eq!(
        run(src),
        "length,name,prototype,m,a,b\nlength,name,prototype,m,a"
    );
}

#[test]
fn a_match_results_groups_is_a_null_prototype_object() {
    // 22.2.7.2 builds `groups` with OrdinaryObjectCreate(null), so it inherits
    // nothing and inspects with the `[Object: null prototype]` tag.
    let src = r#"
        const m = 'abc'.match(/(?<x>b)/);
        console.log(Object.keys(m).join(','), Object.getOwnPropertyNames(m).join(','));
        console.log(m.groups instanceof Object, Object.getPrototypeOf(m.groups));
        console.log(m);
    "#;
    assert_eq!(
        run(src),
        "0,1,index,input,groups 0,1,length,index,input,groups\n\
         false null\n\
         [\n  'b',\n  'b',\n  index: 1,\n  input: 'abc',\n  \
         groups: [Object: null prototype] { x: 'b' }\n]"
    );
}

#[test]
fn a_template_objects_raw_is_a_frozen_non_enumerable_own_property() {
    // GetTemplateObject (13.2.8.4) defines `raw` non-writable, non-enumerable
    // and non-configurable, so it stays out of `Object.keys` while
    // `getOwnPropertyNames` still reports it.
    let src = r#"
        function tag(strings) {
          return [
            Object.keys(strings).join(','),
            Object.getOwnPropertyNames(strings).join(','),
            JSON.stringify(Object.getOwnPropertyDescriptor(strings, 'raw').writable),
            JSON.stringify(Object.getOwnPropertyDescriptor(strings, 'raw').enumerable),
            JSON.stringify(Object.getOwnPropertyDescriptor(strings, 'raw').configurable),
            strings.raw.join('|'),
          ].join('/');
        }
        console.log(tag`a\n${1}b`);
    "#;
    assert_eq!(run(src), "0,1/0,1,length,raw/false/false/false/a\\n|b");
}

#[test]
fn an_arrays_length_is_a_non_enumerable_non_configurable_own_property() {
    // The array exotic's `length` (10.4.2): writable, never enumerated, never
    // configurable — so `delete a.length` reports false and leaves it alone.
    let src = r#"
        const a = [1, 2];
        const d = Object.getOwnPropertyDescriptor(a, 'length');
        console.log([d.value, d.writable, d.enumerable, d.configurable].join(','));
        console.log(delete a.length, a.length);
        console.log(JSON.stringify(Object.getOwnPropertyDescriptors([1])));
    "#;
    assert_eq!(
        run(src),
        "2,true,false,false\n\
         false 2\n\
         {\"0\":{\"value\":1,\"writable\":true,\"enumerable\":true,\"configurable\":true},\
         \"length\":{\"value\":1,\"writable\":true,\"enumerable\":false,\"configurable\":false}}"
    );
}

#[test]
fn named_evaluation_sets_a_function_name_at_every_syntactic_site() {
    // 10.2.9 SetFunctionName, reached from NamedEvaluation at each position the
    // grammar calls for it. Only the `const`/`let`/`var` declarator and the
    // named function expression used to fire; everything else left `.name` as
    // `""`. Every expectation below is `node v26.7.0`'s output.
    let src = r#"
        let h; h = function(){};
        const o = { m: function(){}, a: ()=>{}, g: function*(){}, c: class{} };
        const o2 = { sh(){}, async as(){}, *gen(){} };
        const acc = Object.getOwnPropertyDescriptors({ get g(){return 1}, set s(v){} });
        const k = 'ck'; const sym = Symbol('sd');
        const o3 = { [k]: function(){}, [sym]: ()=>{} };
        class C {
          static s = function(){};
          f = function(){};
          static ['sc'] = function(){};
          get gg(){ return 1 }
          set ss(v){}
        }
        const cd = Object.getOwnPropertyDescriptors(C.prototype);
        function pf(x = function(){}) { return x.name }
        const { dd = function(){} } = {}; const [ ee = ()=>{} ] = [];
        console.log([h.name, o.m.name, o.a.name, o.g.name, o.c.name].join(','));
        console.log([o2.sh.name, o2.as.name, o2.gen.name].join(','));
        console.log([acc.g.get.name, acc.s.set.name, cd.gg.get.name, cd.ss.set.name].join(','));
        console.log([o3.ck.name, o3[sym].name].join(','));
        console.log([C.s.name, new C().f.name, C.sc.name].join(','));
        console.log([pf(), dd.name, ee.name].join(','));
    "#;
    assert_eq!(
        run(src),
        "h,m,a,g,c\n\
         sh,as,gen\n\
         get g,set s,get gg,set ss\n\
         ck,[sd]\n\
         s,f,sc\n\
         x,dd,ee"
    );
}

#[test]
fn named_evaluation_is_syntactic_not_a_check_on_the_value() {
    // `IsAnonymousFunctionDefinition` is a property of the SOURCE, so a
    // property whose value is merely an *expression* that happens to evaluate
    // to a nameless function keeps the empty name — renaming by value would
    // also rewrite `.name` on a function the program still holds elsewhere.
    // `node v26.7.0` prints exactly these.
    let src = r#"
        const anon = (0, function(){});
        const named = function realName(){};
        const obj = {};
        obj.p = function(){};
        console.log(JSON.stringify([
          ({ m: anon }).m.name,
          ({ m: true ? function(){} : 1 }).m.name,
          ({ m: named }).m.name,
          obj.p.name,
          anon.name,
        ]));
    "#;
    assert_eq!(run(src), r#"["","","realName","",""]"#);
}

#[test]
fn a_class_name_is_bound_inside_its_own_body() {
    // 15.7.14 steps 8-17: the class body evaluates in its own environment
    // holding an immutable binding for the class name, initialized to the class
    // BEFORE the static-field initializers of step 32. Both forms used to throw
    // `ReferenceError`, because the only binding was the outer one a class
    // DECLARATION installs after the body has already run — which a class
    // EXPRESSION never gets at all. The binding must not leak outward.
    let src = r#"
        class C { static x = C.m(); static m(){ return 5 } }
        class N { static n = N.name }
        class Seq { static a = 1; static b = Seq.a + 1 }
        const K = class Inner { static self = Inner.name; m(){ return Inner.name } };
        class Outer { static i = class Inner2 { static v = Inner2.name } }
        class F { f = () => F.name }
        console.log([C.x, N.n, Seq.b].join(','));
        console.log([K.self, new K().m(), Outer.i.v, new F().f()].join(','));
        console.log(typeof globalThis.Inner, typeof globalThis.Inner2);
    "#;
    assert_eq!(run(src), "5,N,2\nInner,Inner,Inner2,F\nundefined undefined");
}

// ── UTF-16 code-unit string indexing ────────────────────────────────────────

#[test]
fn string_indices_count_utf16_code_units() {
    // Every expected value measured on `node v26.7.0`. "𝒳" is U+1D4B3: one code
    // point, TWO code units, so a code-point implementation agrees with node on
    // the whole BMP and is off by one for every index past it.
    let src = r#"
        const S = "𝒳", M = "ab𝒳cd";
        console.log([S.length, M.length, "😀🎉".length].join(','));
        console.log([S.charCodeAt(0), S.charCodeAt(1)].join(','));
        console.log([S.codePointAt(0), S.codePointAt(1)].join(','));
        console.log([M.indexOf("c"), M.lastIndexOf("c"), M.indexOf("c", 3)].join(','));
        console.log([M.includes("c", 5), M.startsWith("c", 4), M.endsWith("𝒳", 4)].join(','));
        console.log([M.substring(2,4), M.substr(2,2), M.slice(2,4)].join(','));
        console.log(JSON.stringify(S.padStart(3, "-")));
        console.log([M.search("c"), M.match(/c/).index].join(','));
        console.log((() => { const r = /./g; r.exec(M); r.exec(M); return r.lastIndex; })());
        console.log((() => { let o = -1; M.replace(/c/, (m, i) => { o = i; return m; }); return o; })());
    "#;
    assert_eq!(
        run(src),
        "2,6,4\n\
         55349,56499\n\
         119987,56499\n\
         4,4,4\n\
         false,true,true\n\
         𝒳,𝒳,𝒳\n\
         \"-𝒳\"\n\
         4,4\n\
         2\n\
         4"
    );
}

#[test]
fn char_code_at_and_code_point_at_differ_out_of_range() {
    // They agree on every in-range BMP index, which is why they shared one
    // implementation and one bug: out of range `charCodeAt` is NaN but
    // `codePointAt` is `undefined`, and a negative or infinite position is out
    // of range for both even though `NaN` means 0. `node v26.7.0`.
    let src = r#"
        const S = "abc";
        // `String(...)`, not `join`'s own coercion: `join` renders `undefined`
        // as the empty string, which would hide the very distinction under test.
        for (const v of [-1, NaN, Infinity, -Infinity, 1.7, 0]) {
          console.log([JSON.stringify(S.charAt(v)), String(S.charCodeAt(v)),
                       String(S.codePointAt(v)), String(S.at(v))].join(' '));
        }
    "#;
    assert_eq!(
        run(src),
        "\"\" NaN undefined c\n\
         \"a\" 97 97 a\n\
         \"\" NaN undefined undefined\n\
         \"\" NaN undefined undefined\n\
         \"b\" 98 98 b\n\
         \"a\" 97 97 a"
    );
}

#[test]
fn from_char_code_truncates_to_uint16_and_from_code_point_does_not() {
    // `fromCharCode` takes code UNITS (each argument wrapped to uint16), so a
    // supplementary code point comes out as a different BMP character and a
    // surrogate PAIR of arguments composes. `fromCodePoint` takes whole code
    // points and throws on anything else. `node v26.7.0`.
    let src = r#"
        console.log(String.fromCharCode(0x1D4B3) === "\u{D4B3}");
        console.log(String.fromCharCode(0xD835, 0xDCB3) === "𝒳");
        console.log(String.fromCodePoint(0x1D4B3, 0x41) === "𝒳A");
        console.log(String.fromCharCode(65, 66));
        try { String.fromCodePoint(0x110000); console.log("no throw"); }
        catch (e) { console.log(e.constructor.name); }
    "#;
    assert_eq!(run(src), "true\ntrue\ntrue\nAB\nRangeError");
}

#[test]
fn string_iteration_stays_in_code_points() {
    // The deliberate exception: `[Symbol.iterator]` yields code POINTS, so a
    // spread of an astral character is ONE element even though `.length` is 2.
    // Making this match the index methods would be a regression, not a fix.
    let src = r#"
        console.log([[..."𝒳"].length, Array.from("𝒳").length, "𝒳".length].join(','));
        console.log(JSON.stringify([..."ab𝒳cd"]));
        console.log((() => { let n = 0; for (const c of "😀🎉") n++; return n; })());
    "#;
    assert_eq!(run(src), "1,1,2\n[\"a\",\"b\",\"𝒳\",\"c\",\"d\"]\n2");
}

#[test]
fn splitting_a_surrogate_pair_yields_the_replacement_char() {
    // The documented boundary (BUGS.md, src/utf16.rs). A Rust `String` cannot
    // hold an unpaired surrogate, so cutting a pair in half gives U+FFFD where
    // node gives the lone surrogate. This test PINS the gap rather than
    // asserting node's value, so the day the storage type can represent a lone
    // surrogate this fails loudly and gets updated instead of drifting.
    //
    // What must NOT drift: the unit COUNT is still 1, every surrounding index
    // still lines up, and writing the value to stdout is byte-identical to node
    // (node also emits U+FFFD for a lone surrogate on stdout — verified with
    // `node -e 'process.stdout.write("𝒳".charAt(0))' | xxd` → `ef bf bd`).
    let src = r#"
        const S = "𝒳";
        console.log([S.charAt(0).length, S.slice(0,1).length, S.split("").length].join(','));
        console.log([S.charAt(0) === "�", S.charAt(1) === "�"].join(','));
        console.log([S.charCodeAt(0), S.charCodeAt(1)].join(','));
        process.stdout.write(S.charAt(0));
        console.log("");
    "#;
    // Line 3 is the load-bearing one: reading the units off the INTACT string
    // is exact — only an extracted half degrades.
    assert_eq!(run(src), "1,1,2\ntrue,true\n55349,56499\n\u{FFFD}");
}

#[test]
fn querystring_parse_treats_an_explicit_undefined_separator_as_default() {
    // `body-parser`'s simple urlencoded parser calls
    // `querystring.parse(body, undefined, undefined, { maxKeys: 1000 })`.
    // Coercing an explicit `undefined` to the string "undefined" made it the
    // separator, so nothing split and the entire body became a single key —
    // express 4's `urlencoded({ extended: false })` returned
    // `{"a=1&b=2&c=3":""}`. Expected values from `node v26.7.0`.
    let src = r#"
        const qs = require('querystring');
        console.log(JSON.stringify(qs.parse('a=1&b=2')));
        console.log(JSON.stringify(qs.parse('a=1&b=2', undefined, undefined, { maxKeys: 1000 })));
        console.log(JSON.stringify(qs.parse('a:1;b:2', ';', ':')));
        console.log(qs.stringify({ a: '1', b: '2' }, undefined, undefined));
        console.log(qs.stringify({ a: '1', b: '2' }, ';', ':'));
    "#;
    assert_eq!(
        run(src),
        "{\"a\":\"1\",\"b\":\"2\"}\n\
         {\"a\":\"1\",\"b\":\"2\"}\n\
         {\"a\":\"1\",\"b\":\"2\"}\n\
         a=1&b=2\n\
         a:1;b:2"
    );
}

// ── Buffer encodings that are defined over UTF-16 code units ────────────────

#[test]
fn buffer_utf16le_and_single_byte_encodings_count_code_units() {
    // `utf16le`/`ucs2` is the string's code units written little-endian, and
    // `latin1`/`ascii` take the LOW BYTE of each unit — so an astral character
    // contributes two units in all three, not one code point. `utf16le` had no
    // arm at all and fell through to UTF-8, which is silent corruption rather
    // than a missing feature. Expected values from `node v26.7.0`.
    let src = r#"
        const S = "ab\u{1D4B3}";
        console.log([Buffer.byteLength(S, "utf16le"), Buffer.byteLength(S, "utf8"),
                     Buffer.byteLength(S, "latin1")].join(','));
        console.log(Buffer.from(S, "utf16le").toString("hex"));
        console.log(Buffer.from(S, "utf16le").toString("utf16le") === S);
        console.log(Buffer.from(S, "latin1").toString("hex"));
        console.log(Buffer.from("61006200", "hex").toString("ucs2"));
        console.log(Buffer.from("610062006300", "hex").toString("utf16le", 2));
        console.log([...Buffer.from([65, 255, 128]).toString("ascii")].map(c => c.charCodeAt(0)).join(','));
        console.log([...Buffer.from([65, 255, 128]).toString("latin1")].map(c => c.charCodeAt(0)).join(','));
    "#;
    assert_eq!(
        run(src),
        "8,6,4\n\
         6100620035d8b3dc\n\
         true\n\
         616235b3\n\
         ab\n\
         bc\n\
         65,127,0\n\
         65,255,128"
    );
}

#[test]
fn base64url_is_a_distinct_alphabet_in_both_directions() {
    // `base64url` is not an alias: it encodes with `-_` and no padding. Decoding
    // accepts BOTH alphabets under EITHER name — and getting that wrong was not
    // an error but an EMPTY buffer, because an unrecognized base64 character is
    // skipped rather than rejected. Expected values from `node v26.7.0`.
    let src = r#"
        const b = Buffer.from([251, 255, 190, 1]);
        console.log([b.toString("base64"), b.toString("base64url")].join(' '));
        console.log([Buffer.from([1]).toString("base64"), Buffer.from([1]).toString("base64url")].join(' '));
        console.log([Buffer.from("-_-_", "base64url").toString("hex"),
                     Buffer.from("-_-_", "base64").toString("hex"),
                     Buffer.from("+/+/", "base64url").toString("hex")].join(' '));
        console.log(Buffer.byteLength("-_-_", "base64url"));
    "#;
    assert_eq!(
        run(src),
        "+/++AQ== -_--AQ\n\
         AQ== AQ\n\
         fbffbf fbffbf fbffbf\n\
         3"
    );
}

#[test]
fn buffer_read_arguments_select_a_range_and_an_encoding() {
    // `toString`'s range and `indexOf`/`lastIndexOf`/`includes`'s `byteOffset`
    // and `encoding` were all ignored, so every partial read returned the whole
    // buffer and every search started at 0 with a UTF-8 needle. A negative
    // offset counts back from the end; an empty needle matches AT the offset.
    // Expected values from `node v26.7.0`.
    let src = r#"
        const b = Buffer.from("abcdef");
        console.log([b.toString("utf8", 1, 3), b.toString("hex", 1, 3), b.toString("utf8", 4, 2) === ""].join(' '));
        console.log([b.toString("utf8", -1), b.toString("utf8", 4, 99)].join(' '));
        const c = Buffer.from("abcabc");
        console.log([c.indexOf("b", 2), c.indexOf("b", -2), c.indexOf("62", "hex"), c.indexOf("62", 2, "hex")].join(','));
        console.log([c.lastIndexOf("b"), c.lastIndexOf("b", 2), c.lastIndexOf("b", -4), c.includes("b", 2)].join(','));
        console.log([c.indexOf("", 2), c.indexOf("", 99), c.lastIndexOf("", 2), c.indexOf("a", 99)].join(','));
    "#;
    assert_eq!(
        run(src),
        "bc 6263 true\n\
         abcdef ef\n\
         4,4,1,4\n\
         4,1,1,true\n\
         2,6,2,-1"
    );
}

#[test]
fn buffer_write_and_fill_resolve_their_overloads_like_node() {
    // `write(string[, offset[, length]][, encoding])` and `fill(value[, offset[,
    // end]][, encoding])` decide what each trailing argument MEANS from its
    // runtime type. Two consequences worth pinning: `write` stops at a CHARACTER
    // boundary (2 bytes of `é€` in a 4-byte buffer, not a half-written `€`), and
    // a STRING in `fill`'s `offset` slot is the encoding AND resets the range to
    // the whole buffer, so `fill('41','hex',1,3)` fills all of it rather than
    // `1..3`. An out-of-range `write` offset is a RangeError, not a no-op.
    // Expected values from `node v26.7.0`.
    let src = r#"
        const w = (...a) => { const x = Buffer.alloc(6, 0x2e); const n = x.write(...a); return n + ":" + x.toString("latin1"); };
        console.log([w("abcd"), w("abcd", 2), w("abcd", 1, 2)].join(' '));
        const h = (...a) => { const x = Buffer.alloc(6, 0); const n = x.write(...a); return n + ":" + x.toString("hex"); };
        console.log([h("4142", "hex"), h("4142", 1, "hex"), h("4142", 0, 2, "hex")].join(' '));
        const u = Buffer.alloc(6, 0); console.log(u.write("ab", 0, "utf16le") + ":" + u.toString("hex"));
        const t = Buffer.alloc(4, 0); console.log(t.write("é€") + ":" + t.toString("hex"));
        try { Buffer.alloc(2, 0).write("ab", 5); console.log("no throw"); }
        catch (e) { console.log(e.constructor.name); }
        console.log([Buffer.alloc(4).fill("4142", "hex").toString("hex"),
                     Buffer.alloc(4).fill("QUI=", "base64").toString("hex"),
                     Buffer.alloc(6).fill("ab", "utf16le").toString("hex"),
                     Buffer.alloc(6, 0x2e).fill("41", "hex", 1, 3).toString("hex")].join(' '));
        console.log(Buffer.alloc(6, 0x2e).fill("Z", 2, 4).toString("latin1"));
    "#;
    assert_eq!(
        run(src),
        "4:abcd.. 4:..abcd 2:.ab...\n\
         2:414200000000 2:004142000000 2:414200000000\n\
         4:610062000000\n\
         2:c3a90000\n\
         RangeError\n\
         41424142 41424142 610062006100 414141414141\n\
         ..ZZ.."
    );
}

#[test]
fn buffer_swaps_reverse_groups_in_place() {
    // swap16/32/64 mutate the receiver and return it (so `b.swap16()` changes
    // `b`), and a length that is not a whole number of groups is a RangeError
    // rather than a partial swap. Expected values from `node v26.7.0`.
    let src = r#"
        console.log([Buffer.from([1,2,3,4]).swap16().toString("hex"),
                     Buffer.from([1,2,3,4]).swap32().toString("hex"),
                     Buffer.from([1,2,3,4,5,6,7,8]).swap64().toString("hex")].join(' '));
        const b = Buffer.from([1,2]); console.log([b.swap16() === b, b.toString("hex")].join(','));
        try { Buffer.from([1,2,3]).swap16(); console.log("no throw"); }
        catch (e) { console.log(e.constructor.name + ": " + e.message); }
    "#;
    assert_eq!(
        run(src),
        "02010403 04030201 0807060504030201\n\
         true,0201\n\
         RangeError: Buffer size must be a multiple of 16-bits"
    );
}

#[test]
fn string_decoder_buffers_every_encoding_that_has_a_chunk_boundary() {
    // UTF-8 was the only encoding that held a partial tail. UTF-16LE must hold
    // an odd trailing byte AND a trailing high surrogate; base64 must hold up to
    // two bytes so it emits whole 3-byte groups (`AQID`/`BA==`, never the
    // early-padded `AQI=`/`AwQ=`). `encoding` reports the CANONICAL name, which
    // is what code branching on `decoder.encoding` reads. A dangling odd byte is
    // dropped at `end()` with no replacement char. From `node v26.7.0`.
    let src = r#"
        const { StringDecoder } = require("string_decoder");
        const B = h => Buffer.from(h, "hex");
        console.log([new StringDecoder("ucs2").encoding, new StringDecoder("UTF-8").encoding,
                     new StringDecoder("binary").encoding, new StringDecoder("utf-16le").encoding].join(','));
        const a = new StringDecoder("utf16le");
        console.log([a.write(B("6100620063")), a.write(B("00")), a.end()].join('|'));
        const b = new StringDecoder("utf16le");
        console.log([b.write(B("35d8")).length, b.write(B("b3dc")), b.end()].join('|'));
        const c = new StringDecoder("utf16le");
        console.log([c.write(B("61")), c.end()].join('|') + "<");
        const d = new StringDecoder("base64");
        console.log([d.write(B("0102")), d.write(B("0304")), d.end()].join('|'));
        const e = new StringDecoder("base64url");
        console.log([e.write(B("fbffbe")), e.end()].join('|') + "<");
    "#;
    assert_eq!(
        run(src),
        "utf16le,utf8,latin1,utf16le\n\
         ab|c|\n\
         0|𝒳|\n\
         |<\n\
         |AQID|BA==\n\
         -_--|<"
    );
}

// ── code-unit string ORDER, and the Annex B / ES2024 string globals ─────────

#[test]
fn string_relational_order_is_by_code_unit() {
    // 7.2.13 IsLessThan and the default `sort` comparator order by CODE UNIT.
    // Rust's `str: Ord` is code-point order, and the two disagree on exactly the
    // pairs where an astral character meets a BMP character at or above U+E000
    // (a surrogate is 0xD800..0xE000, so astral sorts BELOW them all).
    // Expected values from `node v26.7.0`.
    let src = r#"
        const A = "\u{1D4B3}";
        console.log([A < "\uFFFF", A < "\uE000", A > "\uD7FF", "\u{10FFFF}" < "\uE000"].join(','));
        console.log(JSON.stringify(["\uFFFF", A, "\uE000", "a"].sort().map(s => s.codePointAt(0))));
        console.log(["b", "a", "B"].sort().join(''));
        console.log([("café" < "cafz"), ("café" < "cagz")].join(','));
    "#;
    assert_eq!(
        run(src),
        "true,true,true,true\n\
         [97,119987,57344,65535]\n\
         Bab\n\
         false,true"
    );
}

#[test]
fn legacy_escape_unescape_and_well_formed_string_methods() {
    // `escape`/`unescape` (Annex B.2.1) work in CODE UNITS, which is what
    // separates `escape` from `encodeURIComponent`: an astral character becomes
    // the two `%uXXXX` escapes of its surrogate pair, not its UTF-8 bytes.
    // `unescape` never throws — a `%` starting no valid escape passes through.
    // `isWellFormed`/`toWellFormed` (ES2024) are exact for every string this
    // runtime can hold, since a Rust `char` cannot be an unpaired surrogate.
    // Expected values from `node v26.7.0`.
    let src = r#"
        console.log([escape("a b+/@*_-.c"), escape("café"), escape("\u{1D4B3}")].join(' '));
        console.log(escape("\x00\x7f\xff"));
        console.log(unescape(escape("a b café \u{1D4B3}")) === "a b café \u{1D4B3}");
        console.log([unescape("%u0041%42%zz%2"), unescape("%")].join(' '));
        console.log([typeof escape, typeof unescape].join(','));
        console.log(["abc".isWellFormed(), "\u{1D4B3}".isWellFormed(), "ab\u{1D4B3}".toWellFormed()].join(','));
    "#;
    assert_eq!(
        run(src),
        "a%20b+/@*_-.c caf%E9 %uD835%uDCB3\n\
         %00%7F%FF\n\
         true\n\
         AB%zz%2 %\n\
         function,function\n\
         true,true,ab𝒳"
    );
}

// ── entry points: file vs `-e` vs stdin ─────────────────────────────────────

/// Run `src` through the built binary at a NON-file entry point, either
/// `node -e <src>` or `node -` with `src` on stdin, and return trimmed stdout.
///
/// `run` above only ever exercises the script-file entry point, and the three
/// entry points are observably different in Node — `__filename`, `module.id`,
/// `process.argv` and `process.execArgv` all carry which one is running. A
/// harness that pins only one of them cannot see a regression in the others.
fn run_at(entry: &str, src: &str, args: &[&str]) -> String {
    use std::process::Stdio;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_node"));
    if entry == "-e" {
        cmd.arg("-e").arg(src);
    } else {
        cmd.arg("-");
    }
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn node binary");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        if entry == "-" {
            stdin.write_all(src.as_bytes()).expect("write stdin");
        }
    }
    let out = child.wait_with_output().expect("wait");
    if !out.status.success() {
        panic!(
            "program failed at entry {entry}:\n--- stderr ---\n{}\n--- stdout ---\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

const ENTRY_PROBE: &str = r#"
    console.log([typeof module, typeof exports, exports === module.exports].join(','));
    console.log([__filename, __dirname, module.id, module.path].join(' '));
    console.log(JSON.stringify(Object.keys(module)));
"#;

#[test]
fn eval_and_stdin_entry_points_report_their_own_names() {
    // `-e` and `-` are DIFFERENT entry points and Node names them differently.
    // Both give the CJS wrapper variables (a UMD header's
    // `typeof module !== 'undefined'` must take the CommonJS branch), with
    // `__dirname` `.` and `module.id` the entry name. `node v26.7.0`.
    let keys = "[\"id\",\"path\",\"exports\",\"filename\",\"loaded\",\"children\",\"paths\"]";
    assert_eq!(
        run_at("-e", ENTRY_PROBE, &[]),
        format!("object,object,true\n[eval] . [eval] .\n{keys}")
    );
    assert_eq!(
        run_at("-", ENTRY_PROBE, &[]),
        format!("object,object,true\n[stdin] . [stdin] .\n{keys}")
    );
}

#[test]
fn a_script_file_entry_point_reports_its_resolved_path() {
    // The file entry point differs from `-e` on every one of these: `__filename`
    // is the RESOLVED path (not the spelling passed), `__dirname` is its
    // directory, and `module.id` is `.` rather than the entry name. `node v26.7.0`.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("entry.js");
    std::fs::write(&path, ENTRY_PROBE).expect("write script");
    let out = run_bounded_out(&path);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.trim_end().lines().collect();
    assert_eq!(lines[0], "object,object,true");
    // `__filename` is the entry script's REALPATH — Node's loader calls
    // `toRealPath` on the main module, so a temp dir reached through a symlink
    // (`/var` → `/private/var` on macOS) reports the target. Canonicalizing the
    // expectation is what makes that assertion, not a way around it: an
    // implementation that skipped the realpath would report the link path and
    // fail here.
    let real = std::fs::canonicalize(&path).expect("canonicalize");
    let want_dir = real.parent().expect("parent").to_string_lossy().to_string();
    assert_eq!(
        lines[1],
        format!("{} {want_dir} . {want_dir}", real.to_string_lossy())
    );
    assert_eq!(
        lines[2],
        "[\"id\",\"path\",\"exports\",\"filename\",\"loaded\",\"children\",\"paths\"]"
    );
}

#[test]
fn runtime_flags_land_in_exec_argv_not_argv() {
    // `process.argv` is `[execPath, entryScript, ...userArgs]` with the RUNTIME's
    // own flags removed — they are `process.execArgv`, and under `-e` the
    // one-liner source is one of them and there is no `argv[1]` at all. Anything
    // reading `process.argv.slice(2)` for its options depends on this split.
    // `node v26.7.0`.
    let src =
        "console.log(JSON.stringify(process.execArgv), JSON.stringify(process.argv.slice(1)))";
    assert_eq!(
        run_at("-e", src, &["z"]),
        format!("[\"-e\",{}] [\"z\"]", serde_json_string(src))
    );
    assert_eq!(run_at("-", src, &["q"]), "[] [\"-\",\"q\"]");
}

/// Minimal JSON string encoding for the one expected value above.
fn serde_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn a_required_module_gets_the_full_module_object() {
    // A `require`d module's `module` used to carry `exports` and nothing else,
    // so `module.id` / `module.filename` / `module.path` / `module.loaded` were
    // all `undefined` there. Node's `id` for a required module is its ABSOLUTE
    // filename (only the entry module's is `.`), and `loaded` is `false` while
    // the body runs. `node v26.7.0`.
    let dir = tempfile::tempdir().expect("temp dir");
    let dep = dir.path().join("dep.js");
    std::fs::write(
        &dep,
        "module.exports = { keys: Object.keys(module), id: module.id, path: module.path,\n\
          filename: module.filename, loaded: module.loaded, same: module.filename === __filename,\n\
          paths0: module.paths[0] };",
    )
    .expect("write dep");
    let main = dir.path().join("main.js");
    std::fs::write(
        &main,
        "const d = require('./dep.js');\n\
         console.log(JSON.stringify(d.keys));\n\
         console.log([d.id === d.filename, d.same, d.loaded, d.path === __dirname].join(','));\n\
         console.log(d.paths0 === __dirname + '/node_modules');",
    )
    .expect("write main");
    let out = run_bounded_out(&main);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        "[\"id\",\"path\",\"exports\",\"filename\",\"loaded\",\"children\",\"paths\"]\n\
         true,true,false,true\n\
         true"
    );
}

// ── round 7: a stack overflow is a catchable error, not an abort ─────────────

/// Every JS call is a Rust recursion (`run_user_func_nt` -> `run_chunk_on` -> a
/// fresh `fusevm::VM` on the stack), so unbounded recursion used to exhaust the
/// OS stack and kill the process: `fatal runtime error: stack overflow`, exit
/// 134, which no `try`/`catch` can observe. V8 throws a catchable `RangeError`.
///
/// `run` panics on a non-zero exit, so a returning abort fails this outright;
/// the assertions then pin the CONSTRUCTOR, the `instanceof` chain and the
/// message. The last two lines matter as much as the first: a guard that fires
/// too eagerly would break ordinary recursion (depth 1000) and a generator body,
/// which runs on its own coroutine stack the thread's bounds say nothing about.
/// Expected values from node v26.7.0.
#[test]
fn unbounded_recursion_throws_instead_of_aborting() {
    let src = r#"
        function f() { return f(); }
        try { f(); console.log('no throw'); } catch (e) {
          console.log(e.constructor.name, e.name, e instanceof RangeError, e instanceof Error, e.message);
        }
        const o = { valueOf() { return this + 1; } };
        try { o + 1; } catch (e) { console.log(e.name, e.message); }
        const p = { toString() { return String(p); } };
        try { String(p); } catch (e) { console.log(e.name, e.message); }
        function* g() { function r(n) { return n <= 0 ? 0 : 1 + r(n - 1); } try { yield r(1000000); } catch (e) { yield e.name + ' ' + e.message; } }
        console.log(g().next().value);
        function d(n) { return n <= 0 ? 0 : 1 + d(n - 1); }
        console.log(d(1000));
        const cyc = [1]; cyc.push(cyc);
        try { cyc.flat(Infinity); } catch (e) { console.log('flat', e.name, e.message); }
        console.log('still running');
    "#;
    assert_eq!(
        run(src),
        "RangeError RangeError true true Maximum call stack size exceeded\n\
         RangeError Maximum call stack size exceeded\n\
         RangeError Maximum call stack size exceeded\n\
         RangeError Maximum call stack size exceeded\n\
         1000\n\
         flat RangeError Maximum call stack size exceeded\n\
         still running"
    );
}

/// `Array.prototype.join` is the one graph walk the language leaves unbounded,
/// so every engine keeps a JoinStack and renders a receiver already being joined
/// as the empty string. node-js had no such cut: `String(a)` on a
/// self-referential array recursed until the process aborted. Only RE-ENTRANCE
/// is cut, not repetition — `[a,a].join('|')` still renders `a` twice.
/// Expected values from node v26.7.0.
#[test]
fn a_cyclic_array_joins_to_the_empty_string_at_the_cycle() {
    let src = r#"
        const a = [1]; a.push(a); a.push(2);
        console.log(JSON.stringify(a.join('-')));
        const b = [1, 2]; const c = [3, b]; b.push(c);
        console.log(JSON.stringify(b.join(',')));
        const d = []; d.push(d);
        console.log(JSON.stringify(String(d)), JSON.stringify(d.toString()), JSON.stringify(`${d}`));
        const e = [1]; e.push(e);
        console.log(JSON.stringify([e, e].join('|')));
        const f = [1];
        console.log(JSON.stringify([f, f].join('|')));
        const g = [1]; g.push(g);
        console.log(JSON.stringify(g.toLocaleString()));
        console.log(JSON.stringify([1, [2, 3], null, undefined, 4].join('-')));
    "#;
    assert_eq!(
        run(src),
        "\"1--2\"\n\
         \"1,2,3,\"\n\
         \"\" \"\" \"\"\n\
         \"1,|1,\"\n\
         \"1|1\"\n\
         \"1,\"\n\
         \"1-2,3---4\""
    );
}

/// A Map/Set rendered its members through `inspect`, which restarts at indent 0,
/// so the depth gate never fired: nesting printed one level too deep, and a
/// self-referential Map or Set recursed until the process aborted. The last two
/// lines pin that a cycle now terminates at all. Expected values from
/// node v26.7.0.
#[test]
fn map_and_set_inspect_at_their_nesting_depth() {
    let src = r#"
        const m4 = new Map([['d', 1]]), m3 = new Map([['c', m4]]), m2 = new Map([['b', m3]]), m1 = new Map([['a', m2]]);
        console.log(m1);
        const s4 = new Set([1]), s3 = new Set([s4]), s2 = new Set([s3]), s1 = new Set([s2]);
        console.log(s1);
        console.log({ a: { b: new Map([['c', new Map([['d', 1]])]]) } });
        console.log(new Map([['k', [1, [2, [3, [4]]]]]]));
        console.log([[[new Map(), new Set(), [], {}]]]);
        const cm = new Map(); cm.set('m', cm);
        console.log(typeof require('util').inspect(cm));
        console.log('survived');
    "#;
    assert_eq!(
        run(src),
        "Map(1) { 'a' => Map(1) { 'b' => Map(1) { 'c' => [Map] } } }\n\
         Set(1) { Set(1) { Set(1) { [Set] } } }\n\
         { a: { b: Map(1) { 'c' => [Map] } } }\n\
         Map(1) { 'k' => [ 1, [ 2, [Array] ] ] }\n\
         [ [ [ Map(0) {}, Set(0) {}, [], {} ] ] ]\n\
         string\n\
         survived"
    );
}

/// The length arithmetic is checked BEFORE the allocation. `'a'.repeat(2**40)`
/// and `new Array(2**32)` used to sit building a 1 TiB string / four billion
/// elements until they were killed, and `new Array(-1)` / `a.length = -1` were
/// accepted outright. What is bounded is the RESULT, so `''.repeat(2**53)` stays
/// legal, and `ToUint32(len) === ToNumber(len)` is the array test, so `'3'` is 3
/// and `-0` is 0. Expected values from node v26.7.0.
#[test]
fn string_and_array_length_limits_throw_rather_than_allocate() {
    let src = r#"
        const t = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } };
        console.log(t(() => 'a'.repeat(2 ** 40)));
        console.log(t(() => 'abc'.padStart(2 ** 40, 'x')));
        console.log(t(() => 'ab'.padEnd(536870889, 'x')));
        console.log(t(() => ''.repeat(2 ** 53).length));
        console.log(t(() => 'ab'.padStart(2 ** 40, '').length));
        console.log(t(() => new Array(-1)));
        console.log(t(() => new Array(1.5)));
        console.log(t(() => new Array(2 ** 32)));
        console.log(t(() => new Array(-0).length), t(() => new Array(3).length), t(() => new Array('x').length));
        console.log(t(() => { const a = []; a.length = -1; return 'set'; }));
        console.log(t(() => { const a = []; a.length = 'x'; return 'set'; }));
        console.log(t(() => { const a = [1]; a.length = 2 ** 32; return 'set'; }));
        console.log(t(() => { const a = []; a.length = '3'; return a.length; }));
        console.log(t(() => { const a = [1, 2, 3]; a.length = 1; return JSON.stringify(a); }));
        console.log(t(() => { const a = []; a.length = { valueOf() { return 2; } }; return a.length; }));
    "#;
    assert_eq!(
        run(src),
        "RangeError: Invalid string length\n\
         RangeError: Invalid string length\n\
         RangeError: Invalid string length\n\
         0\n\
         2\n\
         RangeError: Invalid array length\n\
         RangeError: Invalid array length\n\
         RangeError: Invalid array length\n\
         0 3 1\n\
         RangeError: Invalid array length\n\
         RangeError: Invalid array length\n\
         RangeError: Invalid array length\n\
         3\n\
         [1]\n\
         2"
    );
}

// ── round 7: error SHAPE — the constructor, not just the message ─────────────

/// Every line here was a silent success in node-js: `Object.create(1)` built a
/// normal object, `Object.defineProperty(1, ...)` wrote nothing and returned,
/// `Object.keys(null)` answered `[]`, and `setPrototypeOf` ignored both the
/// prototype type check and the target's extensibility. The passing rows at the
/// end pin that the checks did not over-reject: a primitive IS object-coercible
/// (`Object.keys(1)` is `[]`), a nullish SOURCE to `assign` is skipped, and
/// re-setting the SAME prototype stays legal on a frozen object.
/// Expected values from node v26.7.0.
#[test]
fn object_statics_reject_their_bad_arguments() {
    let src = r#"
        const t = (f) => { try { const r = f(); return typeof r === 'object' && r !== null ? JSON.stringify(r) : String(r); } catch (e) { return e.constructor.name + ': ' + e.message; } };
        console.log(t(() => Object.create(1)));
        console.log(t(() => Object.create('s')));
        console.log(t(() => Object.create(undefined)));
        console.log(t(() => Object.getPrototypeOf(Object.create(null))));
        console.log(t(() => Object.create({ a: 1 }).a));
        console.log(t(() => Object.setPrototypeOf(1, {})));
        console.log(t(() => Object.setPrototypeOf(null, {})));
        console.log(t(() => Object.setPrototypeOf({}, 1)));
        console.log(t(() => Object.setPrototypeOf(Object.freeze({}), {})));
        console.log(t(() => Object.setPrototypeOf(Object.preventExtensions({}), {})));
        console.log(t(() => Object.setPrototypeOf(Object.freeze({}), Object.prototype)));
        console.log(t(() => { const p = { a: 9 }, o = {}; Object.setPrototypeOf(o, p); return o.a; }));
        console.log(t(() => Object.defineProperty(1, 'a', {})));
        console.log(t(() => Object.defineProperty(null, 'a', {})));
        console.log(t(() => Object.defineProperty({}, 'a', 1)));
        console.log(t(() => Object.defineProperty({}, 'a', undefined)));
        console.log(t(() => { const o = {}; Object.defineProperty(o, 'a', { value: 7, enumerable: true }); return o.a; }));
        console.log(t(() => Object.keys(null)));
        console.log(t(() => Object.values(undefined)));
        console.log(t(() => Object.entries(null)));
        console.log(t(() => Object.getOwnPropertyNames(null)));
        console.log(t(() => Object.getOwnPropertySymbols(null)));
        console.log(t(() => Object.getOwnPropertyDescriptor(null, 'a')));
        console.log(t(() => Object.assign(null, {})));
        console.log(t(() => Object.keys(1)));
        console.log(t(() => Object.keys({ a: 1 })));
        console.log(t(() => Object.assign({}, null)));
        console.log(t(() => Object.assign({ a: 1 }, { b: 2 })));
    "#;
    assert_eq!(
        run(src),
        "TypeError: Object prototype may only be an Object or null: 1\n\
         TypeError: Object prototype may only be an Object or null: s\n\
         TypeError: Object prototype may only be an Object or null: undefined\n\
         null\n\
         1\n\
         1\n\
         TypeError: Object.setPrototypeOf called on null or undefined\n\
         TypeError: Object prototype may only be an Object or null: 1\n\
         TypeError: #<Object> is not extensible\n\
         TypeError: #<Object> is not extensible\n\
         {}\n\
         9\n\
         TypeError: Object.defineProperty called on non-object\n\
         TypeError: Object.defineProperty called on non-object\n\
         TypeError: Property description must be an object: 1\n\
         TypeError: Property description must be an object: undefined\n\
         7\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         TypeError: Cannot convert undefined or null to object\n\
         []\n\
         [\"a\"]\n\
         {}\n\
         {\"a\":1,\"b\":2}"
    );
}

/// A symbol has no `ToString` and no `ToNumber`, so it is the one primitive that
/// throws on coercion (7.1.4 step 2, 7.1.17 step 2). node-js rendered
/// `Symbol(desc)` into the result instead. Which message V8 picks is decided by
/// whether the operation is string concatenation. `String(sym)`, `sym.toString()`
/// and a symbol PROPERTY KEY are the documented exceptions and still work.
/// Expected values from node v26.7.0.
#[test]
fn a_symbol_refuses_every_implicit_conversion() {
    let src = r#"
        const t = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } };
        const s = Symbol('d');
        console.log(t(() => s + ''));
        console.log(t(() => '' + s));
        console.log(t(() => `${s}`));
        console.log(t(() => [s].join(',')));
        console.log(t(() => [s].toString()));
        console.log(t(() => s + s));
        console.log(t(() => s + 1));
        console.log(t(() => s * 1));
        console.log(t(() => -s));
        console.log(t(() => s < 1));
        console.log(t(() => Number(s)));
        console.log(t(() => +s));
        console.log(t(() => String(s)));
        console.log(t(() => s.toString()));
        console.log(t(() => s.description));
        console.log(t(() => s === s), t(() => s == 1));
        console.log(t(() => { const o = {}; o[s] = 1; return Object.getOwnPropertySymbols(o).length; }));
        console.log(t(() => JSON.stringify(s)));
        console.log(t(() => [s].map(String).join('|')));
    "#;
    assert_eq!(
        run(src),
        "TypeError: Cannot convert a Symbol value to a string\n\
         TypeError: Cannot convert a Symbol value to a string\n\
         TypeError: Cannot convert a Symbol value to a string\n\
         TypeError: Cannot convert a Symbol value to a string\n\
         TypeError: Cannot convert a Symbol value to a string\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         TypeError: Cannot convert a Symbol value to a number\n\
         Symbol(d)\n\
         Symbol(d)\n\
         d\n\
         true false\n\
         1\n\
         undefined\n\
         Symbol(d)"
    );
}

/// Only an ordinary function has a `[[Construct]]` slot. An arrow, a
/// `function*`, an `async function` and a MethodDefinition are callable but not
/// constructable; node-js ran their bodies and handed back a half-built
/// instance. The message names the callee by source text in V8 and by name here
/// (node-js keeps no spans), so this pins the CLASS, which is the part that
/// diverged. Expected values from node v26.7.0.
#[test]
fn new_on_a_non_constructor_is_a_type_error() {
    let src = r#"
        const t = (f) => { try { return String(f()); } catch (e) { return e.constructor.name + ': ' + e.name; } };
        console.log(t(() => { const g = function* () {}; return new g(); }));
        console.log(t(() => { const a = async function () {}; return new a(); }));
        console.log(t(() => { const a = () => {}; return new a(); }));
        console.log(t(() => { const o = { m() {} }; return new o.m(); }));
        console.log(t(() => { function F() { this.x = 1; } return new F().x; }));
        console.log(t(() => { class C { constructor() { this.y = 2; } } return new C().y; }));
        console.log(t(() => { function F() { this.z = 3; } const B = F.bind(null); return new B().z; }));
        console.log(t(() => new Math.max()));
    "#;
    assert_eq!(
        run(src),
        "TypeError: TypeError\n\
         TypeError: TypeError\n\
         TypeError: TypeError\n\
         TypeError: TypeError\n\
         1\n\
         2\n\
         3\n\
         TypeError: TypeError"
    );
}

/// What a test runner reads off a caught `AssertionError` is `actual`,
/// `expected`, `operator` and `generatedMessage` — and node-js carried only
/// `code`/`message`/`stack`, so all four were `undefined` and
/// `e.constructor.name` said `Error`. The operator is the METHOD name for the
/// strict/deep forms and the OPERATOR for the two loose ones. `Object.keys`
/// order is part of the pin. Expected values from node v26.7.0.
#[test]
fn a_failing_assertion_carries_nodes_whole_property_set() {
    let src = r#"
        const a = require('assert');
        const d = (f) => { try { f(); return 'NO-THROW'; } catch (e) {
          return [e.constructor.name, e.name, e.code, JSON.stringify(e.operator), JSON.stringify(e.actual),
            JSON.stringify(e.expected), e.generatedMessage, JSON.stringify(e.diff),
            JSON.stringify(Object.keys(e)), e instanceof Error].join(' ');
        } };
        console.log(d(() => a.equal(1, 2)));
        console.log(d(() => a.notEqual(1, 1)));
        console.log(d(() => a.strictEqual(1, 2)));
        console.log(d(() => a.notStrictEqual(1, 1)));
        console.log(d(() => a.deepStrictEqual({ a: 1 }, { a: 2 })));
        console.log(d(() => a.ok(0)));
        console.log(d(() => a.fail('boom')));
        console.log(d(() => a.throws(() => {})));
        console.log(d(() => a.strictEqual(1, 2, 'cm')));
        console.log(d(() => a.strictEqual(1, 1)));
    "#;
    assert_eq!(
        run(src),
        "AssertionError AssertionError ERR_ASSERTION \"==\" 1 2 true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"!=\" 1 1 true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"strictEqual\" 1 2 true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"notStrictEqual\" 1 1 true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"deepStrictEqual\" {\"a\":1} {\"a\":2} true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"==\" 0 true true \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"fail\"   false \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"throws\"   false \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         AssertionError AssertionError ERR_ASSERTION \"strictEqual\" 1 2 false \"simple\" [\"generatedMessage\",\"code\",\"actual\",\"expected\",\"operator\",\"diff\"] true\n\
         NO-THROW"
    );
}

// ── JSON.stringify: the replacer function and the toJSON key ─────────────────

/// `JSON.stringify(v, fn)` ignored a FUNCTION second argument entirely — the
/// replacer never ran, so no value was transformed, no key was dropped, and the
/// top-level `("" , value)` call never happened. The array (key-filter) form was
/// the only shape implemented. Expected values from node v26.7.0.
#[test]
fn json_stringify_runs_a_function_replacer() {
    let src = r#"
        const seen = [];
        JSON.stringify({ a: 1, b: { c: 2 } }, function (k, v) {
          seen.push(`${JSON.stringify(k)}@${JSON.stringify(this)}`);
          return v;
        });
        console.log(seen.join(' | '));
        console.log(JSON.stringify({ a: 1, b: 2 }, (k, v) => (typeof v === 'number' ? v * 2 : v)));
        console.log(JSON.stringify([1, 2], (k, v) => (typeof v === 'number' ? v * 2 : v)));
        console.log(JSON.stringify({ a: 1, b: 2 }, (k, v) => (k === 'b' ? undefined : v)));
        console.log(JSON.stringify([1, 2], (k, v) => (k === '0' ? undefined : v)));
        console.log(JSON.stringify(5, (k, v) => v * 2));
        console.log(JSON.stringify({ a: 1, b: 2 }, ['a']));
    "#;
    assert_eq!(
        run(src),
        "\"\"@{\"\":{\"a\":1,\"b\":{\"c\":2}}} | \"a\"@{\"a\":1,\"b\":{\"c\":2}} | \"b\"@{\"a\":1,\"b\":{\"c\":2}} | \"c\"@{\"c\":2}\n\
         {\"a\":2,\"b\":4}\n\
         [2,4]\n\
         {\"a\":1}\n\
         [null,2]\n\
         10\n\
         {\"a\":1}"
    );
}

/// `toJSON` was called with NO arguments (so its `key` parameter read
/// `undefined`) and was then re-applied to its own result, which turned
/// `{toJSON(){return {toJSON(){return 1}}}}` into `1` where V8 answers `{}`.
/// The circular-structure guard has to survive both fixes. Expected values from
/// node v26.7.0 (the message is compared on its first line only — node appends
/// a `--> starting at …` trace this runtime does not build).
#[test]
fn to_json_receives_its_key_and_runs_once() {
    let src = r#"
        console.log(JSON.stringify({ a: { toJSON(k) { return 'K:' + k; } } }));
        console.log(JSON.stringify([{ toJSON(k) { return k; } }]));
        console.log(JSON.stringify({ toJSON(k) { return JSON.stringify(k); } }));
        console.log(JSON.stringify({ toJSON() { return { toJSON() { return 1; } }; } }));
        console.log(JSON.stringify({ d: { toJSON() { return 'x'; } } }, (k, v) => (v === 'x' ? 'y' : v)));
        const c = {}; c.self = c;
        try { JSON.stringify(c); } catch (e) { console.log(e.constructor.name + ': ' + e.message.split('\n')[0]); }
        const g = { get a() { return g; } };
        try { JSON.stringify(g); } catch (e) { console.log(e.constructor.name + ': ' + e.message.split('\n')[0]); }
    "#;
    assert_eq!(
        run(src),
        "{\"a\":\"K:a\"}\n\
         [\"0\"]\n\
         \"\\\"\\\"\"\n\
         {}\n\
         {\"d\":\"y\"}\n\
         TypeError: Converting circular structure to JSON\n\
         TypeError: Converting circular structure to JSON"
    );
}

// ── util.format numeric directives ──────────────────────────────────────────

/// `%d`, `%i` and `%f` are three DIFFERENT conversions (`Number`, `parseInt`,
/// `parseFloat`); one truncating `to_number` served all three, which lost the
/// fraction under `%d`, refused a numeric prefix under `%i`/`%f`, and saturated
/// every non-finite value to `i64::MAX`. `Number.MIN_VALUE` is pinned alongside
/// because it was the smallest NORMAL double, not the smallest subnormal.
/// Expected values from node v26.7.0.
#[test]
fn format_numeric_directives_use_their_own_conversions() {
    let src = r#"
        const util = require('util');
        console.log(util.format('%d|%i|%f', 1.7, 1.7, 1.7));
        console.log(util.format('%d|%i|%f', '3.9abc', '3.9abc', '3.9abc'));
        console.log(util.format('%d|%i|%f', 10n, 10n, 10n));
        console.log(util.format('%d|%i|%f', -0, -0, -0));
        console.log(util.format('%d|%i|%f', 1e21, 1e21, 1e21));
        console.log(util.format('%d|%i|%f', Infinity, Infinity, Infinity));
        console.log(util.format('%d|%i|%f', NaN, NaN, NaN));
        console.log(util.format('%d|%i|%f', {}, {}, {}));
        console.log(util.format('%d|%i|%f', Symbol('s'), Symbol('s'), Symbol('s')));
        console.log(Number.MIN_VALUE, Number.MAX_VALUE, Number.EPSILON);
    "#;
    assert_eq!(
        run(src),
        "1.7|1|1.7\n\
         NaN|3|3.9\n\
         10n|10n|10\n\
         -0|0|0\n\
         1e+21|1|1e+21\n\
         Infinity|NaN|Infinity\n\
         NaN|NaN|NaN\n\
         NaN|NaN|NaN\n\
         NaN|NaN|NaN\n\
         5e-324 1.7976931348623157e+308 2.220446049250313e-16"
    );
}

// ── util.inspect cycle marking ──────────────────────────────────────────────

/// A back-edge was only stopped by the DEPTH limit, so a self-referential object
/// printed three misleading copies of itself ending in `[Object]` instead of
/// naming the cycle. Both ends are marked: `<ref *N>` on the target and
/// `[Circular *N]` on the edge. A merely REPEATED (acyclic) reference must stay
/// unmarked, and the depth limit must still apply to ordinary nesting. Expected
/// values from node v26.7.0.
#[test]
fn inspect_marks_both_ends_of_a_cycle() {
    let src = r#"
        const c = { a: 1 }; c.c = c; console.log(c);
        const arr = [1]; arr.push(arr); console.log(arr);
        const m = new Map(); m.set('m', m); console.log(m);
        const s = new Set(); s.add(s); console.log(s);
        const p = { a: {} }; p.a.up = p; p.b = p; console.log(p);
        const n1 = {}, n2 = {}; n1.n = n2; n2.n = n1; console.log(n1);
        const f = function () {}; f.self = f; console.log(f);
        const shared = { s: 1 }; console.log([shared, shared]);
        console.log({ a: { b: { c: { d: 1 } } } });
    "#;
    assert_eq!(
        run(src),
        "<ref *1> { a: 1, c: [Circular *1] }\n\
         <ref *1> [ 1, [Circular *1] ]\n\
         <ref *1> Map(1) { 'm' => [Circular *1] }\n\
         <ref *1> Set(1) { [Circular *1] }\n\
         <ref *1> { a: { up: [Circular *1] }, b: [Circular *1] }\n\
         <ref *1> { n: { n: [Circular *1] } }\n\
         <ref *1> [Function: f] { self: [Circular *1] }\n\
         [ { s: 1 }, { s: 1 } ]\n\
         { a: { b: { c: [Object] } } }"
    );
}

// ── Symbol.hasInstance ──────────────────────────────────────────────────────

/// `instanceof` walked the prototype chain unconditionally: `Symbol.hasInstance`
/// did not even exist as a symbol, so every custom-membership class silently
/// answered `false`. The handler is looked up on the class-static side table
/// (following `extends`) as well as the property map, is called with the
/// right-hand side as `this`, propagates a throw, and — per GetMethod — treats
/// only `undefined`/`null` as absent. Expected values from node v26.7.0.
#[test]
fn instanceof_consults_symbol_has_instance() {
    let src = r#"
        console.log(typeof Symbol.hasInstance, String(Symbol.hasInstance));
        class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }
        class SubEven extends Even {}
        console.log(2 instanceof Even, 3 instanceof Even, 4 instanceof SubEven, 5 instanceof SubEven);
        const oddish = { [Symbol.hasInstance](x) { return x > 2; } };
        console.log([1, 2, 3, 4].filter((x) => x instanceof oddish).join(','));
        const f = function () {}; Object.defineProperty(f, Symbol.hasInstance, { value: () => true });
        console.log(1 instanceof f);
        class Boom { static [Symbol.hasInstance]() { throw new Error('boom'); } }
        try { 1 instanceof Boom; } catch (e) { console.log(e.message); }
        for (const v of [1, 's', true, {}, null, undefined]) {
          const o = { [Symbol.hasInstance]: v };
          try { console.log(1 instanceof o); } catch (e) { console.log(e.constructor.name + ': ' + e.message); }
        }
        class A {} class B extends A {}
        console.log(new B() instanceof A, new A() instanceof B, [] instanceof Array, ({}) instanceof Object);
    "#;
    assert_eq!(
        run(src),
        "symbol Symbol(Symbol.hasInstance)\n\
         true false true false\n\
         3,4\n\
         true\n\
         boom\n\
         TypeError: number 1 is not a function\n\
         TypeError: string \"s\" is not a function\n\
         TypeError: boolean true is not a function\n\
         TypeError: object is not a function\n\
         TypeError: Right-hand side of 'instanceof' is not callable\n\
         TypeError: Right-hand side of 'instanceof' is not callable\n\
         true false true true"
    );
}

// ── class static initialization blocks (ES2022) ─────────────────────────────

/// `class C { static { … } }` was a hard `SyntaxError: bad member key Punct("{")`
/// — the whole script failed to parse. The block runs once at class-definition
/// time with `this` bound to the constructor, interleaved with the static field
/// initializers in source order, and must leave NO property behind (which is
/// what `getOwnPropertyNames` pins here). `static` as an ordinary member name
/// still has to parse as one. Expected values from node v26.7.0.
#[test]
fn class_static_blocks_run_against_the_constructor() {
    let src = r#"
        class A {
          static x = 1;
          static #p = 3;
          static { this.viaThis = 5; }
          static { A.y = A.x + 1; A.viaPrivate = A.#p; }
          static m() { return 7; }
          static { A.viaMethod = A.m(); }
        }
        console.log(A.x, A.y, A.viaThis, A.viaPrivate, A.viaMethod);
        console.log(Object.getOwnPropertyNames(A).join(','), Object.keys(A).join(','));
        class Scoped { static { let v = 1; { let v = 2; void v; } Scoped.v = v; } }
        console.log(Scoped.v);
        const C = class { static { this.n = 9; } };
        class Outer { static { Outer.inner = class { static { this.deep = 1; } }; } }
        console.log(C.n, Outer.inner.deep);
        class Named { static(){ return 'call'; } static static = 'field'; }
        console.log(new Named().static(), Named.static);
    "#;
    assert_eq!(
        run(src),
        "1 2 5 3 7\n\
         length,name,prototype,m,x,viaThis,y,viaPrivate,viaMethod x,viaThis,y,viaPrivate,viaMethod\n\
         1\n\
         9 1\n\
         call field"
    );
}

// ── Math argument coercion / expanded ISO years ─────────────────────────────

/// Every `Math` function coerces with `ToNumber`, and `ToNumber` of a BigInt is
/// a TypeError — but the argument reader took a BigInt's magnitude instead, so
/// `Math.max(1n)` quietly answered `1`. `Math.random` never reads an argument
/// and is the one function that must NOT throw. Expected values from node
/// v26.7.0.
#[test]
fn math_refuses_bigint_arguments() {
    let src = r#"
        for (const e of ['Math.abs(1n)', 'Math.max(1n,2)', 'Math.min(1n)', 'Math.hypot(1n)',
                         'Math.pow(2n,2)', 'Math.atan2(1,2n)', 'Math.imul(1n,2)', 'Math.clz32(1n)']) {
          try { eval(e); console.log('NO-THROW ' + e); }
          catch (err) { console.log(err.constructor.name + ': ' + err.message); }
        }
        console.log(typeof Math.random(1n));
        console.log(Math.max(1, 2, 3), Math.hypot(3, 4), Math.min());
    "#;
    let want = "TypeError: Cannot convert a BigInt value to a number\n".repeat(8);
    assert_eq!(run(src), format!("{want}number\n3 5 Infinity"));
}

/// `toISOString` formatted the year with a plain four-wide pad, which counts the
/// sign inside the width and never emits `+`: year -1 printed `-001` and year
/// 275760 printed unsigned. Outside 0..=9999 the spec uses the EXPANDED form —
/// an explicit sign and exactly six digits. Expected values from node v26.7.0.
#[test]
fn iso_string_expands_years_outside_four_digits() {
    let src = r#"
        console.log(new Date(8.64e15).toISOString());
        console.log(new Date(-8.64e15).toISOString());
        console.log(new Date(Date.UTC(-1, 0, 1)).toISOString());
        console.log(new Date(Date.UTC(10000, 0, 1)).toISOString());
        console.log(new Date(Date.UTC(9999, 11, 31)).toISOString());
        console.log(new Date(Date.UTC(0, 0, 1)).toISOString());
        console.log(new Date(0).toISOString(), JSON.stringify(new Date(-62198755200000)));
    "#;
    assert_eq!(
        run(src),
        "+275760-09-13T00:00:00.000Z\n\
         -271821-04-20T00:00:00.000Z\n\
         -000001-01-01T00:00:00.000Z\n\
         +010000-01-01T00:00:00.000Z\n\
         9999-12-31T00:00:00.000Z\n\
         1900-01-01T00:00:00.000Z\n\
         1970-01-01T00:00:00.000Z \"-000001-01-01T00:00:00.000Z\""
    );
}

// ── Proxy ───────────────────────────────────────────────────────────────────

/// Every trap that intercepts a property operation, each one observed through
/// the operator that triggers it rather than through `Reflect` (which would only
/// prove `Reflect` and the trap agree, not that `p.a` reaches the handler at
/// all). The last two lines pin the two enumerations that are NOT `ownKeys`
/// alone: `Object.keys` and `for-in` additionally filter by each key's
/// `[[GetOwnProperty]]`, so a key the `ownKeys` trap invents survives only
/// because `getOwnPropertyDescriptor` calls it enumerable. Expected values from
/// node v26.7.0.
#[test]
fn proxy_property_traps_intercept_the_operators() {
    let src = r#"
        const log = [];
        const t = { a: 1, b: 2 };
        const p = new Proxy(t, {
          get(tt, k)            { log.push('get:' + String(k)); return tt[k] * 10; },
          set(tt, k, v)         { log.push('set:' + String(k)); tt[k] = v; return true; },
          has(tt, k)            { log.push('has:' + String(k)); return k === 'ghost' || k in tt; },
          deleteProperty(tt, k) { log.push('del:' + String(k)); delete tt[k]; return true; },
          ownKeys()             { return ['a', 'invented']; },
          getOwnPropertyDescriptor(tt, k) {
            return { value: 9, enumerable: k !== 'invented' || true, configurable: true };
          },
        });
        console.log(p.a, p.b);
        p.c = 3;
        console.log('ghost' in p, 'nope' in p, t.c);
        console.log(delete p.b, t.b);
        console.log(log.join('|'));
        console.log(Object.keys(p).join(','));
        const seen = [];
        for (const k in p) seen.push(k);
        console.log(seen.join(','));
    "#;
    assert_eq!(
        run(src),
        "10 20\n\
         true false 3\n\
         true undefined\n\
         get:a|get:b|set:c|has:ghost|has:nope|del:b\n\
         a,invented\n\
         a,invented"
    );
}

/// A handler with no traps at all must be observationally invisible: every
/// operation forwards to the target. This is the property that makes a proxy
/// usable as a wrapper, and it is the one most easily broken by an interception
/// that forgets its fallback — each line below reaches the target through a
/// DIFFERENT funnel (read, write, `in`, enumeration, JSON, spread, iteration,
/// call, construct, `instanceof`, `Array.isArray`, brand). Expected values from
/// node v26.7.0.
#[test]
fn a_trapless_proxy_is_transparent() {
    let src = r#"
        const p = new Proxy({ a: 1, b: 2 }, {});
        p.c = 3;
        console.log(p.a, 'b' in p, Object.keys(p).join(','), JSON.stringify(p));
        console.log(JSON.stringify({ ...p }), p instanceof Object);
        const arr = new Proxy([1, 2, 3], {});
        console.log(arr.length, Array.isArray(arr), [...arr].join(','), arr.map(x => x * 2).join(','));
        console.log(JSON.stringify(arr), Object.prototype.toString.call(arr));
        function add(x, y) { return x + y; }
        const pf = new Proxy(add, {});
        console.log(typeof pf, pf(2, 3), pf.name, pf.length);
        class C { constructor(n) { this.n = n; } m() { return 'm' + this.n; } }
        const pc = new Proxy(C, {});
        const inst = new pc(7);
        console.log(inst.n, inst.m(), inst instanceof C);
    "#;
    assert_eq!(
        run(src),
        "1 true a,b,c {\"a\":1,\"b\":2,\"c\":3}\n\
         {\"a\":1,\"b\":2,\"c\":3} true\n\
         3 true 1,2,3 2,4,6\n\
         [1,2,3] [object Array]\n\
         function 5 add 2\n\
         7 m7 true"
    );
}

/// `apply` / `construct` / `getPrototypeOf` / `setPrototypeOf` / `defineProperty`
/// / `isExtensible`, plus `typeof` on a proxy of a function (10.5 installs the
/// `[[Call]]` slot only for a callable target, so the answer is the TARGET's).
/// Expected values from node v26.7.0.
#[test]
fn proxy_call_construct_and_prototype_traps() {
    let src = r#"
        const pf = new Proxy(function (a, b) { return a + b; },
                             { apply(t, self, args) { return t(...args) * 2; } });
        console.log(pf(1, 2), typeof pf);
        class C { constructor(x) { this.x = x; } }
        const pc = new Proxy(C, { construct(t, args) { return new t(args[0] + 100); } });
        console.log(new pc(1).x);
        const pp = new Proxy({}, { getPrototypeOf() { return Array.prototype; } });
        console.log(Object.getPrototypeOf(pp) === Array.prototype, pp instanceof Array);
        const swallow = new Proxy({}, { setPrototypeOf() { return true; } });
        Object.setPrototypeOf(swallow, Array.prototype);
        console.log(Object.getPrototypeOf(swallow) === Object.prototype);
        const pd = new Proxy({}, { defineProperty(t, k, d) { t[k] = d.value * 2; return true; } });
        Object.defineProperty(pd, 'z', { value: 5, enumerable: true, configurable: true, writable: true });
        console.log(pd.z);
        console.log(Object.isExtensible(new Proxy(Object.freeze({}), {})),
                    Object.isExtensible(new Proxy({}, { isExtensible: () => true })));
    "#;
    assert_eq!(
        run(src),
        "6 function\n\
         101\n\
         true true\n\
         true\n\
         10\n\
         false true"
    );
}

/// `Proxy.revocable`: after `revoke()` EVERY operation throws, naming the one it
/// attempted, while `typeof` still answers from the slot fixed at creation.
/// Calling `revoke` twice is a no-op, not a second teardown. Construction
/// rejects a non-object target or handler, and `Proxy` has no `[[Call]]` slot.
/// Expected messages verbatim from node v26.7.0.
#[test]
fn revoked_proxies_and_construction_errors() {
    let src = r#"
        const show = f => { try { f(); console.log('NO-THROW'); }
                            catch (e) { console.log(e.constructor.name + ': ' + e.message); } };
        show(() => Proxy({}, {}));
        show(() => new Proxy(1, {}));
        show(() => new Proxy({}, 'x'));
        show(() => new Proxy({}));
        const { proxy, revoke } = Proxy.revocable({ a: 1 }, {});
        console.log(proxy.a);
        revoke();
        revoke();
        console.log(typeof proxy);
        show(() => proxy.a);
        show(() => 'a' in proxy);
        show(() => Object.keys(proxy));
        const fr = Proxy.revocable(function () {}, {});
        fr.revoke();
        console.log(typeof fr.proxy);
        show(() => fr.proxy());
        show(() => { const q = new Proxy({}, { get: 1 }); return q.a; });
    "#;
    assert_eq!(
        run(src),
        "TypeError: Constructor Proxy requires 'new'\n\
         TypeError: Cannot create proxy with a non-object as target or handler\n\
         TypeError: Cannot create proxy with a non-object as target or handler\n\
         TypeError: Cannot create proxy with a non-object as target or handler\n\
         1\n\
         object\n\
         TypeError: Cannot perform 'get' on a proxy that has been revoked\n\
         TypeError: Cannot perform 'has' on a proxy that has been revoked\n\
         TypeError: Cannot perform 'ownKeys' on a proxy that has been revoked\n\
         function\n\
         TypeError: Cannot perform 'apply' on a proxy that has been revoked\n\
         TypeError: '1' returned for property 'get' of object '#<Object>' is not a function"
    );
}

/// A proxy used as a PROTOTYPE. `OrdinaryGet` forwards down the chain with the
/// ORIGINAL receiver, so the trap's third argument is the child — and a getter
/// re-dispatched through `Reflect.get(t, k, receiver)` sees the child as `this`.
/// The method-call form is a separate funnel from the property read and was
/// broken independently. `class D extends <proxy>` links `D.prototype` through
/// the proxy's `prototype` read, and `super(...)` runs `[[Construct]]` on it.
/// Expected values from node v26.7.0.
#[test]
fn proxy_in_a_prototype_chain_and_as_a_superclass() {
    let src = r#"
        const base = { get who() { return this.name; } };
        const p = new Proxy(base, { get(t, k, r) { return Reflect.get(t, k, r); } });
        console.log(Object.create(p, { name: { value: 'X' } }).who);
        const proto = new Proxy({}, { get(t, k) { return k === 'greet' ? () => 'hi' : undefined; } });
        console.log(Object.create(proto).greet());
        class B { constructor() { this.b = 1; } m() { return 'm'; } }
        class D extends new Proxy(B, {}) { constructor() { super(); this.d = 2; } }
        const d = new D();
        console.log(d.b, d.d, d.m(), d instanceof B, d instanceof D);
        const nested = new Proxy(new Proxy({ v: 1 }, { get: (t, k) => t[k] + 10 }),
                                 { get: (t, k) => t[k] * 2 });
        console.log(nested.v);
    "#;
    assert_eq!(run(src), "X\nhi\n1 2 m true true\n22");
}

/// Symbol keys reach the traps as SYMBOLS, not as node-js's internal `@@…`
/// strings — a trap that switches on `typeof k` (the common membrane guard) has
/// to see `'symbol'`. `Reflect.ownKeys` reports the symbol half of the trap's
/// answer, and `Object.prototype.toString` brands from a `Symbol.toStringTag`
/// read through the `get` trap. `hasOwnProperty` is `[[GetOwnProperty]]`, so it
/// consults the DESCRIPTOR trap rather than `has`. Expected values from node
/// v26.7.0.
#[test]
fn proxy_traps_receive_symbol_keys() {
    let src = r#"
        const S = Symbol('s');
        const seen = [];
        const ps = new Proxy({}, {
          get(t, k) { seen.push('get:' + typeof k); return t[k]; },
          set(t, k, v) { seen.push('set:' + typeof k); t[k] = v; return true; },
        });
        ps[S] = 1; void ps[S]; void ps.a;
        console.log(seen.join(','));
        const po = new Proxy({ [S]: 1, a: 2 }, {});
        console.log(Object.getOwnPropertySymbols(po).length, Reflect.ownKeys(po).map(String).join('|'));
        const tagged = new Proxy({}, { get: (t, k) => (k === Symbol.toStringTag ? 'Zed' : undefined) });
        console.log(Object.prototype.toString.call(tagged));
        const own = new Proxy({}, {
          getOwnPropertyDescriptor: (t, k) =>
            (k === 'z' ? { value: 1, configurable: true, enumerable: true } : undefined),
        });
        console.log(Object.prototype.hasOwnProperty.call(own, 'z'),
                    Object.prototype.hasOwnProperty.call(own, 'q'));
    "#;
    assert_eq!(
        run(src),
        "set:symbol,get:symbol,get:string\n\
         1 a|Symbol(s)\n\
         [object Zed]\n\
         true false"
    );
}

/// A `get` trap that lies about `length` must be honored by iteration, because
/// `Array.prototype[Symbol.iterator]` is generic: it reads `length` and then
/// each index through `[[Get]]`. node-js models that method as a thunk bound to
/// the array it was read off, which would have walked the target and ignored the
/// trap entirely. `JSON.stringify` reads through `[[Get]]` for the same reason.
/// Expected values from node v26.7.0.
#[test]
fn iteration_through_a_proxy_honors_the_get_trap() {
    let src = r#"
        const short = new Proxy([1, 2, 3], { get: (t, k) => (k === 'length' ? 2 : t[k]) });
        console.log(short.length, [...short].join(','), JSON.stringify(short));
        const doubled = new Proxy([1, 2], { get: (t, k) => (k === 'length' ? t.length : t[k] * 5) });
        console.log([...doubled].join(','), JSON.stringify(doubled));
        const custom = new Proxy({}, { get: (t, k) => (k === Symbol.iterator ? function* () { yield 7; yield 8; } : undefined) });
        console.log([...custom].join(','));
        console.log(JSON.stringify(Object.assign({}, new Proxy({ x: 1, y: 2 }, { get: (t, k) => t[k] * 3 }))));
        console.log(JSON.stringify({ ...new Proxy({ x: 1 }, { get: (t, k) => t[k] + 1 }) }));
    "#;
    assert_eq!(
        run(src),
        "2 1,2 [1,2]\n\
         5,10 [5,10]\n\
         7,8\n\
         {\"x\":3,\"y\":6}\n\
         {\"x\":2}"
    );
}

// ── Array.prototype.sort: order, undefined placement, comparison count ───────

/// `sort` must not hand `undefined` to the comparator, must place it after
/// every defined value, and must be stable. Expected values from node v26.7.0:
/// `[3,undefined,1].sort((x,y)=>x-y)` is `[1,3,undefined]` after exactly ONE
/// call. The insertion sort this replaced compared `undefined` like any other
/// value, called the comparator twice, and left `[3,undefined,1]`.
#[test]
fn sort_keeps_undefined_out_of_the_comparator_and_is_stable() {
    let src = r#"
        const a = [3, undefined, 1];
        let calls = 0;
        a.sort((x, y) => { calls++; return x - y; });
        console.log(JSON.stringify(a), calls, a.length);
        console.log(JSON.stringify([3, undefined, 1, undefined, 2].sort()));
        const pairs = [{k:1,i:0},{k:0,i:1},{k:1,i:2},{k:0,i:3}];
        console.log(JSON.stringify(pairs.sort((x, y) => x.k - y.k)));
        console.log(JSON.stringify([1, 2, 3].sort(() => NaN)));
        console.log(JSON.stringify([10, 9, 1, 2].sort()));
    "#;
    assert_eq!(
        run(src),
        "[1,3,null] 1 3\n\
         [1,2,3,null,null]\n\
         [{\"k\":0,\"i\":1},{\"k\":0,\"i\":3},{\"k\":1,\"i\":0},{\"k\":1,\"i\":2}]\n\
         [1,2,3]\n\
         [1,10,2,9]"
    );
}

/// The comparison COUNT is the regression guard: `sort` was quadratic, so 4096
/// reversed elements cost 8.4M comparator calls and 200k elements did not
/// finish inside 120s (node v26.7.0 sorts those in 70ms). A merge sort of
/// n = 4096 is bounded by n*log2(n) = 49152; the bound below leaves headroom
/// for a different O(n log n) algorithm but a return to O(n²) fails it by two
/// orders of magnitude. Counting, not timing, so it cannot go flaky on a busy
/// CI machine.
#[test]
fn sort_comparison_count_stays_linearithmic() {
    let src = r#"
        const n = 4096;
        const a = [];
        for (let i = 0; i < n; i++) a.push(n - i);
        let calls = 0;
        a.sort((x, y) => { calls++; return x - y; });
        console.log(a[0], a[n - 1], calls <= n * 12, calls < 100000);
    "#;
    assert_eq!(run(src), "1 4096 true true");
}

/// A typed array sorts numerically by default and shares `Array`'s merge sort
/// for a user comparator; a comparator returning NaN means "keep this order"
/// (23.2.4.1 SortCompare step 3), which the old `<= 0.0` break inverted into a
/// swap. Expected values from node v26.7.0.
#[test]
fn typed_array_sort_matches_node() {
    let src = r#"
        console.log(new Int32Array([5, 1, 4, 2, 3]).sort((a, b) => a - b).join(','));
        console.log(new Uint8Array([10, 9, 1]).sort().join(','), [10, 9, 1].sort().join(','));
        console.log(new Float64Array([3, 1, 2]).sort(() => NaN).join(','));
        const big = new Int32Array(2048);
        for (let i = 0; i < 2048; i++) big[i] = (i * 7919) % 2048;
        big.sort((a, b) => a - b);
        console.log(big[0], big[1024], big[2047]);
    "#;
    assert_eq!(
        run(src),
        "1,2,3,4,5\n\
         1,9,10 1,10,9\n\
         3,1,2\n\
         0 1024 2047"
    );
}

// ── loop scopes: what the per-iteration copy is for, and what it costs ───────

/// `for (let i …)` re-binds per iteration so a closure made in one pass keeps
/// that pass's value, and a block opens a scope for its lexical declarations.
/// Both are skipped when nothing in the subtree can capture — so the cases that
/// CAN capture have to keep working, and the cases that cannot must not start
/// leaking their bindings. Expected values from node v26.7.0.
#[test]
fn loop_and_block_scopes_survive_the_capture_analysis() {
    let src = r#"
        const fns = [];
        for (let i = 0; i < 3; i++) fns.push(() => i);
        console.log(fns.map(f => f()).join(','));
        const g = [];
        for (let j = 0; j < 3; j++) { const k = j * 2; g.push(() => k + j); }
        console.log(g.map(f => f()).join(','));
        let i = 'outer';
        for (let i = 0; i < 3; i++) {}
        console.log(i);
        for (let q = 0; q < 3; q++) {}
        console.log(typeof q);
        { let x = 1; }
        console.log(typeof x);
        { var v = 5; }
        console.log(v);
        let s = 0;
        for (let n = 0; n < 3; n++) { let s = 100; s++; }
        console.log(s);
        const vf = [];
        for (var w = 0; w < 3; w++) vf.push(() => w);
        console.log(vf.map(f => f()).join(','));
        let acc = 0;
        outer: for (let a = 0; a < 4; a++) { if (a === 2) continue outer; if (a === 3) break outer; acc += a; }
        console.log(acc);
        let t = 0;
        for (let a = 0; a < 3; a++) { try { if (a === 1) throw new Error('x'); t += 1; } catch (e) { t += 10; } }
        console.log(t);
        const nf = [];
        for (let a = 0; a < 2; a++) for (let b = 0; b < 2; b++) nf.push(() => a * 10 + b);
        console.log(nf.map(f => f()).join(','));
        for (let a = 0; a < 1; a++) { eval('var ev = 7'); }
        console.log(ev);
        function* gen() { for (let a = 0; a < 3; a++) yield () => a; }
        console.log([...gen()].map(f => f()).join(','));
    "#;
    assert_eq!(
        run(src),
        "0,1,2\n\
         0,3,6\n\
         outer\n\
         undefined\n\
         undefined\n\
         5\n\
         0\n\
         3,3,3\n\
         1\n\
         12\n\
         0,1,10,11\n\
         7\n\
         0,1,2"
    );
}

// ── locals in frame slots (src/slots.rs) ────────────────────────────────────

/// A local that no other chunk can name is addressed as a fusevm frame slot
/// rather than looked up in the scope chain. The cases here are the ones where
/// that rewrite could go wrong: parameters (which arrive in the environment and
/// are copied in), defaults and rest, `arguments`, shadowing, a closure that
/// captures a loop variable, a `try` block (its own chunk, on its own frame),
/// `typeof`, and calling through a slotted binding. Expected values from node
/// v26.7.0.
#[test]
fn slotted_locals_keep_their_scope_semantics() {
    let src = r#"
        function add(a, b) { return a + b; }
        console.log(add(2, 3), add("a", "b"));
        function def(a, b = 7) { return a + b; }
        console.log(def(1), def(1, 2));
        function rest(a, ...r) { return a + r.length; }
        console.log(rest(1, 2, 3));
        function args() { return arguments.length; }
        console.log(args(1, 2, 3));
        function shadow(x) { let y = x * 2; { let y = 9; x += y; } return x + y; }
        console.log(shadow(3));
        function loopvar() {
            let t = 0;
            for (const v of [1, 2, 3]) t += v;
            for (const k in { a: 1, b: 2 }) t += k.length;
            return t;
        }
        console.log(loopvar());
        function sw(n) { let r = "?"; switch (n) { case 1: r = "one"; break; default: r = "many"; } return r; }
        console.log(sw(1), sw(5));
        function withTry(n) {
            let acc = 0;
            try { acc += n; throw new Error("x"); } catch (e) { acc += 10; } finally { acc += 100; }
            return acc;
        }
        console.log(withTry(1));
        function capturing(n) { const fns = []; for (let i = 0; i < n; i++) fns.push(() => i); return fns.map(f => f()).join(","); }
        console.log(capturing(3));
        function* gen(n) { let i = 0; while (i < n) yield i++; }
        console.log([...gen(3)].join(","));
        const slotted = Math.abs;
        console.log(slotted(-4), typeof slotted, typeof neverDeclared);
        let counter = 0;
        for (let i = 0; i < 4; i++) counter += i;
        console.log(counter, typeof counter);
    "#;
    assert_eq!(
        run(src),
        "5 ab\n\
         8 3\n\
         3\n\
         3\n\
         18\n\
         8\n\
         one many\n\
         111\n\
         0,1,2\n\
         0,1,2\n\
         4 function undefined\n\
         6 number"
    );
}

/// `++`/`--` on a slot the compiler proved holds a Number lowers to a native
/// add rather than the `NUM_STEP` builtin (which exists to keep `x++` on a
/// BigInt a BigInt). The pairs below are what that rewrite must not change:
/// prefix vs postfix values, a fractional counter, a negative start, the
/// float boundary at 2^53, and a `let` that is reassigned — which drops out of
/// the numeric set and keeps the builtin. Expected values from node v26.7.0.
#[test]
fn increment_on_a_numeric_slot_matches_node() {
    let src = r#"
        let i = 0;
        console.log(i++, i, ++i, i, i--, i, --i, i);
        let f = 0.5;
        console.log(f++, f, ++f);
        let n = -1;
        console.log(n++, n);
        let big = 9007199254740991;
        big++;
        console.log(big);
        let mixed = 1;
        mixed = "2";
        mixed++;
        console.log(mixed, typeof mixed);
        let b = 1n;
        b++;
        console.log(b, typeof b);
        let total = 0;
        for (let k = 0; k < 4; k++) total += k;
        console.log(total);
    "#;
    assert_eq!(
        run(src),
        "0 1 2 2 2 1 0 0\n\
         0.5 1.5 2.5\n\
         -1 0\n\
         9007199254740992\n\
         3 number\n\
         2n bigint\n\
         6"
    );
}

// ── Constructor-side class inheritance (15.7.14 step 6.d) ────────────────────

/// `class B extends A` links the CONSTRUCTORS, not just the prototypes:
/// `B.[[Prototype]]` is `A`. Statics already resolved through an internal
/// parent pointer, but the link itself was invisible, so `getPrototypeOf(B)`
/// answered `Function.prototype` and a chain that bottomed out in a BUILTIN
/// (`class D extends Array {}`) could not reach that builtin's statics at all.
/// Expected values from node v26.7.0.
#[test]
fn class_constructor_side_inherits_from_its_parent() {
    let src = r#"
        class A { static sm(){ return "A.sm" } static sf = 1; }
        class B extends A {}
        class C extends B {}
        console.log(Object.getPrototypeOf(B) === A, Object.getPrototypeOf(C) === B);
        console.log(Object.getPrototypeOf(A) === Function.prototype);  // base class
        console.log(B.sm(), B.sf, C.sm(), C.sf);                       // still inherited
        console.log(Object.getPrototypeOf(B.prototype) === A.prototype);
        class D extends Array {}
        console.log(typeof D.from, Object.getPrototypeOf(D) === Array);
        console.log(JSON.stringify(D.from([1,2])));
    "#;
    assert_eq!(
        run(src),
        "true true\ntrue\nA.sm 1 A.sm 1\ntrue\nfunction true\n[1,2]"
    );
}

// ── CopyDataProperties over a string source (7.3.25) ─────────────────────────

/// `{...str}` spreads the string's index properties, because ToObject gives a
/// String exotic owning one enumerable property per UTF-16 code UNIT (10.4.3) —
/// so an astral character contributes TWO. The spread path only walked heap
/// objects, so every string source contributed nothing and `{..."ab"}` was `{}`.
/// The other primitives box to an object with no own enumerable properties and
/// must stay no-ops. Expected values from node v26.7.0.
#[test]
fn object_spread_of_a_string_copies_its_index_properties() {
    let src = r#"
        console.log(JSON.stringify({..."ab"}));
        // An astral char is TWO code units, so it contributes two keys — the
        // count is what distinguishes code-unit from code-POINT indexing. Only
        // the key count is asserted: the two VALUES are lone surrogates, which
        // this runtime's UTF-8 string storage cannot hold (see `toWellFormed`).
        console.log(Object.keys({..."a\u{1F600}b"}).length);
        console.log(JSON.stringify({..."é"}));            // non-ASCII BMP round-trips
        console.log(JSON.stringify({...""}));
        console.log(JSON.stringify({...[1,2]}));          // array source unchanged
        console.log(JSON.stringify({...1, ...true, ...null, ...undefined}));
        console.log(JSON.stringify({a:0, ..."xy"}));
    "#;
    assert_eq!(
        run(src),
        "{\"0\":\"a\",\"1\":\"b\"}\n\
         4\n\
         {\"0\":\"é\"}\n\
         {}\n\
         {\"0\":1,\"1\":2}\n\
         {}\n\
         {\"0\":\"x\",\"1\":\"y\",\"a\":0}"
    );
}

// ── String.prototype.normalize (22.1.3.15, UAX-15) ───────────────────────────

/// `normalize` used to return the receiver unchanged and merely validate the
/// form argument, making all four forms no-ops. The consequence was silent and
/// bad: the standard way to compare Unicode text for canonical equivalence
/// answered `false` for two spellings of the same character, and NFKC never
/// folded a compatibility character. The pairs below are chosen so an identity
/// implementation fails every line. Expected values from node v26.7.0.
#[test]
fn normalize_actually_normalizes() {
    let src = r#"
        const composed = "\u00C5";        // LATIN CAPITAL LETTER A WITH RING ABOVE
        const decomposed = "\u0041\u030A"; // A + COMBINING RING ABOVE
        console.log(composed.length, decomposed.length);
        console.log(composed.normalize("NFC").length, composed.normalize("NFD").length);
        console.log(decomposed.normalize("NFC").length, decomposed.normalize("NFD").length);
        console.log(composed.normalize("NFC") === decomposed.normalize("NFC"));
        console.log(composed.normalize() === decomposed.normalize());  // default NFC
        console.log("\uFB01".normalize("NFKC"), "\uFB01".normalize("NFKC").length);
        console.log("\uFB01".normalize("NFC").length);                 // NFC does NOT fold
        console.log("\u2460".normalize("NFKD"));                       // circled 1 -> "1"
        console.log("abc".normalize("NFD"));                           // ASCII unaffected
        try { "a".normalize("NFX") } catch (e) { console.log(e.constructor.name) }
    "#;
    assert_eq!(
        run(src),
        "1 2\n\
         1 2\n\
         1 2\n\
         true\n\
         true\n\
         fi 2\n\
         1\n\
         1\n\
         abc\n\
         RangeError"
    );
}

// ── util.inspect: maxArrayLength, typed arrays, JSON short escapes ───────────

/// `util.inspect` formats at most `maxArrayLength` (100) array elements and
/// collapses the rest into `... N more items`. The cap was missing, so a
/// 120-element array printed all 120 — and because the grid's column width is
/// computed from what is SHOWN, every column was also one character wider than
/// node's. The tail is not an element: node drops it from the grid so it
/// neither widens a column nor fills a cell, then re-appends it on its own
/// line. Expected values from node v26.7.0.
#[test]
fn array_inspect_caps_at_one_hundred_items() {
    let src = r#"
        const a = []; for (let i = 0; i < 120; i++) a.push(i);
        console.log(a);
        const b = []; for (let i = 0; i < 101; i++) b.push(0);
        console.log(b.length, String(console.log) === String(console.log));
        const exact = []; for (let i = 0; i < 100; i++) exact.push(1);
        console.log(String(exact.length));
    "#;
    let out = run(src);
    // The grid is sized from the 100 SHOWN entries (max width 2), not from 119.
    assert!(
        out.contains("   0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11,"),
        "columns must be sized to the shown entries, got:\n{out}"
    );
    assert!(
        out.contains("  96, 97, 98, 99,\n  ... 20 more items\n]"),
        "the tail belongs on its own line after the grid, got:\n{out}"
    );
    // Singular/plural and the exact-boundary case (no tail at exactly 100).
    assert!(!out.contains("more item\n"), "20 items must be plural");
}

/// A typed array inspects as `Uint8Array(3) [ 1, 2, 3 ]` — constructor, length,
/// then the elements laid out as an array's. It used to fall through to the
/// generic object arm and print the `{ length, byteLength, byteOffset,
/// BYTES_PER_ELEMENT }` bookkeeping instead of the contents. Expected values
/// from node v26.7.0.
#[test]
fn typed_array_inspects_as_its_contents() {
    let src = r#"
        console.log(new Uint8Array([1,2,3]));
        console.log(new Int32Array([1,2]));
        console.log(new Float64Array(2));
        console.log(new Uint8Array(0));
        console.log([new Uint8Array([1])]);
        console.log({t: new Uint8Array([1,2])});
        console.log(new Uint8Array([1,2,3]).subarray(1));
    "#;
    assert_eq!(
        run(src),
        "Uint8Array(3) [ 1, 2, 3 ]\n\
         Int32Array(2) [ 1, 2 ]\n\
         Float64Array(2) [ 0, 0 ]\n\
         Uint8Array(0) []\n\
         [ Uint8Array(1) [ 1 ] ]\n\
         { t: Uint8Array(2) [ 1, 2 ] }\n\
         Uint8Array(2) [ 2, 3 ]"
    );
}

/// `Object.assign` filled a plain-object target in place and matched ONLY that
/// shape, so every other target silently copied nothing and was returned
/// untouched — no error, just a missing property. An array target is the common
/// case. Expected values from node v26.7.0.
#[test]
fn object_assign_reaches_a_non_object_target() {
    let src = r#"
        const a = Object.assign([1,2], {extra: 9});
        console.log(a.extra, JSON.stringify(Object.keys(a)), a.length);
        console.log(a);
        const b = Object.assign([1,2], {2: 3});       // an INDEX key extends it
        console.log(JSON.stringify(b), b.length);
        const f = Object.assign(function(){}, {tag: 't'});
        console.log(f.tag, typeof f);
        console.log(JSON.stringify(Object.assign({}, {a:1}, {b:2})));  // unchanged
        console.log(Object.assign([1,2]) === undefined);
    "#;
    assert_eq!(
        run(src),
        "9 [\"0\",\"1\",\"extra\"] 2\n\
         [ 1, 2, extra: 9 ]\n\
         [1,2,3] 3\n\
         t function\n\
         {\"a\":1,\"b\":2}\n\
         false"
    );
}

/// QuoteJSONString (25.5.2.2) names SIX short escapes. Backspace and form feed
/// were missing and fell through to the `\uXXXX` arm, so `JSON.stringify("\b")`
/// produced `"\u0008"` where node produces `"\b"`. Both parse back to the same
/// string, so the difference is invisible to a round trip and shows up only as
/// a byte mismatch against a fixture or checksum. Expected from node v26.7.0.
#[test]
fn json_stringify_uses_every_short_escape() {
    let src = r#"
        console.log(JSON.stringify("\b\f\n\r\t\"\\"));
        console.log(JSON.stringify("\u0000\u0001\u001f"));   // no short form
        console.log(JSON.stringify({ "\b": "\f" }));
        console.log(JSON.parse(JSON.stringify("\b\f")) === "\b\f");
        console.log(JSON.stringify("\u007f"));               // DEL is NOT escaped
    "#;
    assert_eq!(
        run(src),
        "\"\\b\\f\\n\\r\\t\\\"\\\\\"\n\
         \"\\u0000\\u0001\\u001f\"\n\
         {\"\\b\":\"\\f\"}\n\
         true\n\
         \"\u{7f}\""
    );
}

// ── Date: component setters, TimeClip, ToDateString ──────────────────────────

/// Every Date component SETTER was missing — only `setTime` existed — so a Date
/// could be read but never modified field-wise: `d.setUTCFullYear(2000)` threw
/// `date.setUTCFullYear is not a function`. Each setter takes its own field plus
/// every lower-order one in its group (date 0..2, time 3..6), defaults the rest
/// from the current time value, normalizes overflow, and returns the new time
/// value. The NaN split is the spec's: `setFullYear` on an Invalid Date starts
/// from the epoch and so REVIVES it (21.4.4.21 step 2), every other setter
/// leaves it invalid. Expected values from node v26.7.0 under TZ=UTC.
#[test]
fn date_component_setters_match_node() {
    let src = r#"
        const d = new Date(0);  console.log(d.setUTCFullYear(2000), d.toISOString());
        const e = new Date(NaN);console.log(e.setUTCFullYear(2000), e.toISOString());
        const f = new Date(NaN);console.log(f.setUTCMonth(5), String(f));
        const g = new Date(0);  console.log(g.setUTCMonth(13), g.toISOString());
        const h = new Date(0);  console.log(h.setUTCDate(32), h.toISOString());
        const i = new Date(0);  console.log(i.setUTCHours(1,2,3,4), i.toISOString());
        const j = new Date(0);  console.log(j.setUTCFullYear(2000,5,15), j.toISOString());
        const l = new Date(0);  console.log(l.setUTCMilliseconds(1.9), l.toISOString());
        const n = new Date(0);  console.log(n.setUTCFullYear(NaN), String(n));
        const o = new Date(0);  console.log(o.setMinutes(30,15), o.toISOString());
        const p = new Date(0);  console.log(p.setSeconds(61), p.toISOString());
        const q = new Date(0);  console.log(q.setDate(0), q.toISOString());
        const r = new Date(0);  console.log(r.setUTCFullYear(2020,1,29), r.toISOString());
        const m = new Date(0);  console.log(m.getYear(), m.setYear(99), m.toISOString());
    "#;
    assert_eq!(
        run(src),
        "946684800000 2000-01-01T00:00:00.000Z\n\
         946684800000 2000-01-01T00:00:00.000Z\n\
         NaN Invalid Date\n\
         34214400000 1971-02-01T00:00:00.000Z\n\
         2678400000 1970-02-01T00:00:00.000Z\n\
         3723004 1970-01-01T01:02:03.004Z\n\
         961027200000 2000-06-15T00:00:00.000Z\n\
         1 1970-01-01T00:00:00.001Z\n\
         NaN Invalid Date\n\
         1815000 1970-01-01T00:30:15.000Z\n\
         61000 1970-01-01T00:01:01.000Z\n\
         -86400000 1969-12-31T00:00:00.000Z\n\
         1582934400000 2020-02-29T00:00:00.000Z\n\
         70 915148800000 1999-01-01T00:00:00.000Z"
    );
}

/// TimeClip (21.4.1.31): a time value beyond ±8.64e15 ms is not representable
/// and becomes NaN. It was not applied anywhere, so `new Date(8.64e15 + 1)`
/// kept the raw number and printed a real date (`Sat, 13 Sep 275760 …`) where
/// node prints `Invalid Date` — the boundary every date-range check relies on.
/// Expected values from node v26.7.0.
#[test]
fn date_clips_out_of_range_time_values() {
    let src = r#"
        console.log(new Date(8.64e15).toISOString());
        console.log(String(new Date(8.64e15 + 1)));
        console.log(String(new Date(-8.64e15 - 1)));
        console.log(new Date(-8.64e15).toISOString());
        console.log(String(new Date(Infinity)), String(new Date(-Infinity)));
        const k = new Date(0);
        console.log(k.setTime(8.64e15 + 1), String(k));
        console.log(new Date(1.5).getTime());   // truncates toward zero
    "#;
    assert_eq!(
        run(src),
        "+275760-09-13T00:00:00.000Z\n\
         Invalid Date\n\
         Invalid Date\n\
         -271821-04-20T00:00:00.000Z\n\
         Invalid Date Invalid Date\n\
         NaN Invalid Date\n\
         1"
    );
}

/// `Date.prototype.toString` is ToDateString (21.4.4.41) — the `toDateString`
/// half, a space, then the `toTimeString` half. It used to answer the RFC-7231
/// header form, which is what `toUTCString` is for, so `String(date)` and any
/// template interpolation printed `Thu, 01 Jan 1970 00:00:00 GMT` instead of
/// node's form. `toTimeString` was missing outright. Expected from node v26.7.0
/// under TZ=UTC.
#[test]
fn date_to_string_is_not_the_utc_header_form() {
    let src = r#"
        const d = new Date(0);
        console.log(d.toString());
        console.log(String(d));
        console.log(`${d}`);
        console.log(d.toTimeString());
        console.log(d.toDateString());
        console.log(d.toUTCString());          // the RFC form, unchanged
        console.log(String(new Date(NaN)), new Date(NaN).toTimeString());
    "#;
    assert_eq!(
        run(src),
        "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)\n\
         Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)\n\
         Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)\n\
         00:00:00 GMT+0000 (Coordinated Universal Time)\n\
         Thu Jan 01 1970\n\
         Thu, 01 Jan 1970 00:00:00 GMT\n\
         Invalid Date Invalid Date"
    );
}

// ── const bindings are immutable (8.5.2 SetMutableBinding) ──────────────────

/// Assigning to a `const` used to SUCCEED, silently, through every form:
/// `const c = 1; c = 2` left `c` as 2 with no error, so code node rejects ran
/// on with a mutated constant. It is a RUNTIME error, not a parse error — the
/// spec puts it in SetMutableBinding — so a `try` around it must CATCH it,
/// which is what every case below checks. `=`, compound assignment and
/// `++`/`--` each reach the store by a different path in the compiler, and the
/// `++` fast path for a slot proved to hold a Number bypassed the other two, so
/// all three are pinned here. Expected values from node v26.7.0.
#[test]
fn assignment_to_a_const_throws() {
    let src = r#"
        function t(f){ try { return String(f()) } catch(e) { return e.constructor.name + ": " + e.message } }
        console.log(t(()=>{ const c = 1; c = 2; return c }));
        console.log(t(()=>{ const c = 1; c += 1; return c }));
        console.log(t(()=>{ const c = 1; c -= 1; return c }));
        console.log(t(()=>{ const c = 1; c++; return c }));
        console.log(t(()=>{ const c = 1; ++c; return c }));
        console.log(t(()=>{ const c = 1; c--; return c }));
        console.log(t(()=>{ const c = 1; { c = 2 } return c }));   // inner block
        console.log(t(()=>{ const c = 1; return (()=>{ c = 2 })() }));  // inner fn
        console.log(t(()=>{ const c = 1; c ||= 2; return c }));
    "#;
    let expect = "TypeError: Assignment to constant variable.";
    assert_eq!(run(src), [expect; 9].join("\n"));
}

/// The flip side, and the more important half: everything that is NOT an
/// assignment to the binding must keep working. A false positive here would be
/// a REGRESSION — code that runs today would start throwing — so each of these
/// is a shape that looks like a const store but is not one.
#[test]
fn const_enforcement_does_not_over_reject() {
    let src = r#"
        const o = {}; o.x = 1; o.x++;                 // mutating the OBJECT is fine
        const a = [1]; a.push(2); a[0] = 9;
        console.log(o.x, JSON.stringify(a));
        for (const i of [1,2]) { }                    // fresh binding per iteration
        for (const k in {a:1,b:2}) { }
        let out = "";
        for (const v of [1,2,3]) out += v;
        console.log(out);
        const c = 1; { let c = 2; c = 3; console.log(c) }   // inner LET shadows
        { const c = 5; console.log(c) }
        console.log(c);
        function f(c) { c = 7; return c }             // a PARAMETER is mutable
        console.log(f(1));
        const g = () => { let n = 0; n++; return n };  // let inside is mutable
        console.log(g());
        let l = 1; l = 2; l += 1; l++; console.log(l);
        const { p } = { p: 1 }; console.log(p);
        const [q] = [2]; console.log(q);
        console.log(typeof c);
    "#;
    assert_eq!(
        run(src),
        "2 [9,2]\n\
         123\n\
         3\n\
         5\n\
         1\n\
         7\n\
         1\n\
         4\n\
         1\n\
         2\n\
         number"
    );
}

// ── shared compiled regex engine, per-object regex state ────────────────────

/// The compiled engine behind a regex is cached and SHARED between every
/// evaluation of the same pattern (see `regexp::compiled` — it removed 85% of
/// the wall time of `require("express")`). What must not be shared is anything
/// observable: each evaluation of a literal is a distinct `RegExp` object with
/// its own `lastIndex`, and two patterns differing only in flags are two
/// engines. This test is the guard on that: it fails if the cache is ever keyed
/// too loosely, or if the object itself starts being reused. Expected values
/// from node v26.7.0.
#[test]
fn regex_objects_stay_distinct_while_the_engine_is_shared() {
    let src = r#"
        function mk(){ return /a/g }
        const r1 = mk(), r2 = mk();
        console.log(r1 === r2, r1.lastIndex, r2.lastIndex);
        r1.test("aaa"); console.log(r1.lastIndex, r2.lastIndex);
        r1.test("aaa"); console.log(r1.lastIndex, r2.lastIndex);
        r2.test("aaa"); console.log(r1.lastIndex, r2.lastIndex);
        const a = /x/g, b = /x/g; console.log(a === b);
        // Same source, DIFFERENT flags: two engines, and the flags must stick.
        const c = /x/, d = /x/g;
        console.log(JSON.stringify([c.flags, d.flags, c.global, d.global]));
        console.log(JSON.stringify([/a/gi.flags, /a/gi.ignoreCase, /a/g.ignoreCase]));
        console.log(JSON.stringify(["AaA".replace(/a/gi,"-"), "AaA".replace(/a/g,"-")]));
        const e = new RegExp("y","g"), f = new RegExp("y","g");
        console.log(e === f);
        e.lastIndex = 3; console.log(e.lastIndex, f.lastIndex);
        // A literal inside a loop is re-evaluated: a fresh lastIndex each time.
        let out = [];
        for (let i = 0; i < 3; i++) { const g = /z/g; g.test("zz"); out.push(g.lastIndex); }
        console.log(JSON.stringify(out));
    "#;
    assert_eq!(
        run(src),
        "false 0 0\n\
         1 0\n\
         2 0\n\
         2 1\n\
         false\n\
         [\"\",\"g\",false,true]\n\
         [\"gi\",true,false]\n\
         [\"---\",\"A-A\"]\n\
         false\n\
         3 0\n\
         [1,1,1]"
    );
}

// ── array holes (elided elements) ────────────────────────────────────────────
//
// A hole is NOT a stored `undefined`: it reads back as one, but it is not an
// own property, and half the array methods are spec'd to skip it. The two facts
// are what these tests pin, together with the `<N empty items>` rendering.

#[test]
fn array_holes_are_not_own_properties() {
    // `[1,,3]` — an ELIDED element is not an own property, so it is absent
    // from `in`, `Object.keys/values/entries`, `hasOwnProperty`, `for…in`,
    // spread-into-object and `getOwnPropertyDescriptor`, while still READING
    // back as `undefined`.
    let src = r##"
        const a = [1,,3];
        console.log(1 in a, 0 in a, 2 in a);
        console.log(JSON.stringify(Object.keys(a)));
        console.log(JSON.stringify(Object.values(a)));
        console.log(JSON.stringify(Object.entries(a)));
        console.log(a.hasOwnProperty(1), a.hasOwnProperty(0), a.hasOwnProperty('length'));
        console.log(a.length, JSON.stringify(a));
        console.log(JSON.stringify(Object.getOwnPropertyNames(a)));
        console.log(a.propertyIsEnumerable(1), a.propertyIsEnumerable(0));
        const seen = []; for (const k in a) seen.push(k); console.log(JSON.stringify(seen));
        console.log(JSON.stringify(Object.assign({}, a)));
        console.log(JSON.stringify({...a}));
        console.log(Object.getOwnPropertyDescriptor(a, 1));
        console.log(JSON.stringify(Object.getOwnPropertyDescriptor(a, 0)));
    "##;
    let expected = [
        "false true true",
        "[\"0\",\"2\"]",
        "[1,3]",
        "[[\"0\",1],[\"2\",3]]",
        "false true true",
        "3 [1,null,3]",
        "[\"0\",\"2\",\"length\"]",
        "false true",
        "[\"0\",\"2\"]",
        "{\"0\":1,\"2\":3}",
        "{\"0\":1,\"2\":3}",
        "undefined",
        "{\"value\":1,\"writable\":true,\"enumerable\":true,\"configurable\":true}",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_holes_survive_structural_mutation() {
    // A hole tracks its element through every structural mutation —
    // `push`/`pop`/`shift`/`unshift`, the change-by-copy methods, `copyWithin`,
    // `fill`, a `length` grow, a write past the end — and `util.inspect`
    // renders each maximal run as `<N empty items>`.
    let src = r##"
        const a=[1,,3];
        console.log(a.at(1), a[1]);
        console.log([1,,3,,5].toSorted());
        console.log([1,,3].toReversed());
        console.log([1,,3].with(0,9));
        console.log([1,,3,4].toSpliced(1,1));
        console.log([1,,3].copyWithin(0,1));
        console.log([1,2,3,,5].fill(9,1,3));
        console.log(Array(200));
        console.log([1,...Array(3),5]);
        const big=[]; big[150]=1; console.log(big);
        console.log(Array.of(1,undefined,3));
        console.log([1,,3].keys().next(), [...[1,,3].keys()]);
        console.log([...[1,,3].entries()]);
        console.log(JSON.stringify([1,,3], null, 0));
        console.log([1,,3].lastIndexOf(undefined), [1,,3].indexOf(3));
        console.log([1,,3].findLast(x=>true), [1,,3].findLastIndex(x=>x===undefined));
        const f=[1,,3]; Object.freeze(f); console.log(Object.keys(f), f.length);
        console.log(Array.isArray([1,,3]), [1,,3].constructor === Array);
        let z=[1,,3]; z.push(4); console.log(z, Object.keys(z));
        let y=[1,,3]; y.pop(); console.log(y, Object.keys(y), y.length);
        let x=[,,3]; x.shift(); console.log(x, Object.keys(x));
        let w=[1,,3]; w.unshift(0); console.log(w, Object.keys(w));
        console.log(structuredClone([1,,3]));
    "##;
    let expected = [
        "undefined undefined",
        "[ 1, 3, 5, undefined, undefined ]",
        "[ 3, undefined, 1 ]",
        "[ 9, undefined, 3 ]",
        "[ 1, 3, 4 ]",
        "[ <1 empty item>, 3, 3 ]",
        "[ 1, 9, 9, <1 empty item>, 5 ]",
        "[ <200 empty items> ]",
        "[ 1, undefined, undefined, undefined, 5 ]",
        "[ <150 empty items>, 1 ]",
        "[ 1, undefined, 3 ]",
        "{ value: 0, done: false } [ 0, 1, 2 ]",
        "[ [ 0, 1 ], [ 1, undefined ], [ 2, 3 ] ]",
        "[1,null,3]",
        "-1 2",
        "3 1",
        "[ '0', '2' ] 3",
        "true true",
        "[ 1, <1 empty item>, 3, 4 ] [ '0', '2', '3' ]",
        "[ 1, <1 empty item> ] [ '0' ] 2",
        "[ <1 empty item>, 3 ] [ '1' ]",
        "[ 0, 1, <1 empty item>, 3 ] [ '0', '1', '3' ]",
        "[ 1, <1 empty item>, 3 ]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_hole_iteration_methods_skip_them() {
    // The `HasProperty`-spec'd iteration methods
    // (`forEach`/`map`/`filter`/`some`/`every`/`reduce`/`flat`/`sort`) SKIP a
    // hole; the `Get`-spec'd ones (`for…of`, spread, `join`, `Array.from`,
    // `includes`) see the `undefined` it reads back as.
    let src = r##"
        console.log(Array(3).every(x=>false), Array(3).some(x=>true));
        console.log(Array(3).map((x,i)=>i));
        console.log(Array.from({length:3}));
        console.log(Array.from(Array(3), (x,i)=>i));
        console.log([,,3].join('|'), [1,,].join('|'));
        const [p,,q] = [1,2,3]; console.log(p,q);
        const [r,s] = [1,,3]; console.log(r,s);
        function f(...a){return a} console.log(f(...[1,,3]));
        console.log(Math.max(...[1,,3]));
        console.log([1,,3].toLocaleString());
        console.log(Object.freeze([1,,3]));
        console.log([].concat([1,,3], 4, [,5]));
        console.log(new Set([1,,3]));
        console.log(new Map([[1,2]]));
        console.log([1,,3].flat(0));
        console.log([[1,,3]].flat());
        console.log(Array(3).fill().map((_,i)=>i));
        console.log(Array(3).join('-'));
        let a=Array(3); a[1]=7; console.log(a, Object.keys(a));
        let b=[1,,3]; b[1]=7; console.log(b, Object.keys(b), 1 in b);
        let c=[1,,3]; delete c[0]; console.log(c, Object.keys(c));
        console.log([1,,3].reduce((p,q)=>p+q, 0));
        console.log(Array(3).reduce((p,q)=>p+q, 0));
        try { Array(3).reduce((p,q)=>p+q) } catch(e) { console.log(e.constructor.name, e.message) }
        console.log(JSON.stringify({a:[1,,3]}));
        console.log([1,,3].sort((x,y)=>0));
        console.log([undefined,,1].sort());
    "##;
    let expected = [
        "true false",
        "[ <3 empty items> ]",
        "[ undefined, undefined, undefined ]",
        "[ 0, 1, 2 ]",
        "||3 1|",
        "1 3",
        "1 undefined",
        "[ 1, undefined, 3 ]",
        "NaN",
        "1,,3",
        "[ 1, <1 empty item>, 3 ]",
        "[ 1, <1 empty item>, 3, 4, <1 empty item>, 5 ]",
        "Set(3) { 1, undefined, 3 }",
        "Map(1) { 1 => 2 }",
        "[ 1, 3 ]",
        "[ 1, 3 ]",
        "[ 0, 1, 2 ]",
        "--",
        "[ <1 empty item>, 7, <1 empty item> ] [ '1' ]",
        "[ 1, 7, 3 ] [ '0', '1', '2' ] true",
        "[ <2 empty items>, 3 ] [ '2' ]",
        "4",
        "0",
        "TypeError Reduce of empty array with no initial value",
        "{\"a\":[1,null,3]}",
        "[ 1, 3, <1 empty item> ]",
        "[ 1, undefined, <1 empty item> ]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_holes_read_back_as_undefined() {
    // The two groups side by side on one array, plus `delete` punching a hole,
    // `Array(n)` being all holes, and `JSON.stringify` writing `null` for each.
    let src = r##"
        const a = [1,,3];
        console.log(1 in a, 0 in a, 2 in a);
        console.log(JSON.stringify(Object.keys(a)));
        console.log(JSON.stringify(Object.values(a)));
        console.log(JSON.stringify(Object.entries(a)));
        console.log(a.hasOwnProperty(1), a.hasOwnProperty(0));
        console.log(a.length);
        let c=0; a.forEach(()=>c++); console.log('forEach', c);
        console.log(JSON.stringify(a.map(x=>x*2)));
        console.log(JSON.stringify(a.filter(()=>true)));
        console.log(a.reduce((p,q)=>p+q));
        console.log(JSON.stringify([...a]));
        console.log(a.join('-'));
        console.log(JSON.stringify(Array.from(a)));
        console.log(JSON.stringify(a));
        console.log(a.indexOf(undefined), a.includes(undefined));
        const b = Array(3);
        console.log(JSON.stringify(Object.keys(b)), b.length, JSON.stringify(b));
        const d = [1,2,3]; delete d[1];
        console.log(d.length, JSON.stringify(Object.keys(d)), 1 in d);
        const e = [1,,];
        console.log(e.length, JSON.stringify(Object.keys(e)));
        console.log(JSON.stringify([3,,1].sort()));
        console.log(JSON.stringify(Object.keys([3,,1].sort())));
        console.log(JSON.stringify([1,,3].flat()));
    "##;
    let expected = [
        "false true true",
        "[\"0\",\"2\"]",
        "[1,3]",
        "[[\"0\",1],[\"2\",3]]",
        "false true",
        "3",
        "forEach 2",
        "[2,null,6]",
        "[1,3]",
        "4",
        "[1,null,3]",
        "1--3",
        "[1,null,3]",
        "[1,null,3]",
        "-1 true",
        "[] 3 [null,null,null]",
        "3 [\"0\",\"2\"] false",
        "2 [\"0\"]",
        "[1,3,null]",
        "[\"0\",\"1\"]",
        "[1,3]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_holes_render_as_empty_items() {
    // `util.inspect` hole rendering: consecutive holes group into one `<N empty
    // items>` entry, at every nesting level.
    let src = r##"
        console.log([1,,3]);
        console.log(Array(3));
        console.log([,,]);
        console.log([1,,,,5]);
        console.log([,1]);
        console.log(new Array(5).fill(0));
        const a=[1,2,3]; delete a[1]; console.log(a);
        console.log([1,,3].map(x=>x*2));
        console.log([3,,1].sort());
        console.log([1,,3].slice(0,2));
        console.log([1,,3].concat([4,,6]));
        console.log([1,,3].reverse());
        console.log([[1,,3]]);
        console.log({x:[1,,3]});
        const b=[1]; b.length=4; console.log(b, Object.keys(b));
        const c=[]; c[3]='x'; console.log(c, Object.keys(c), c.length);
        console.log([1,,3].splice(1,1));
        const d=[1,,3,,5]; console.log(d.splice(1,2), d, Object.keys(d));
        console.log([...[1,,3]]);
        for (const x of [1,,3]) console.log('of', x);
        console.log([1,,3].entries().next().value);
        console.log(Array.from([1,,3]));
        console.log([1,,3].find(x=>x===undefined));
        console.log([1,,3].findIndex(x=>x===undefined));
        console.log([1,,3].every(x=>x!==undefined));
        console.log([1,,3].some(x=>x===undefined));
        console.log([1,,3].flat(), [[1,,3],,4].flat());
        console.log([1,,3].flatMap(x=>[x]));
        console.log([1,,3].reduceRight((p,q)=>p+q));
        console.log([1,,3].toString());
        console.log(String([1,,3]));
    "##;
    let expected = [
        "[ 1, <1 empty item>, 3 ]",
        "[ <3 empty items> ]",
        "[ <2 empty items> ]",
        "[ 1, <3 empty items>, 5 ]",
        "[ <1 empty item>, 1 ]",
        "[ 0, 0, 0, 0, 0 ]",
        "[ 1, <1 empty item>, 3 ]",
        "[ 2, <1 empty item>, 6 ]",
        "[ 1, 3, <1 empty item> ]",
        "[ 1, <1 empty item> ]",
        "[ 1, <1 empty item>, 3, 4, <1 empty item>, 6 ]",
        "[ 3, <1 empty item>, 1 ]",
        "[ [ 1, <1 empty item>, 3 ] ]",
        "{ x: [ 1, <1 empty item>, 3 ] }",
        "[ 1, <3 empty items> ] [ '0' ]",
        "[ <3 empty items>, 'x' ] [ '3' ] 4",
        "[ <1 empty item> ]",
        "[ <1 empty item>, 3 ] [ 1, <1 empty item>, 5 ] [ '0', '2' ]",
        "[ 1, undefined, 3 ]",
        "of 1",
        "of undefined",
        "of 3",
        "[ 0, 1 ]",
        "[ 1, undefined, 3 ]",
        "undefined",
        "1",
        "true",
        "false",
        "[ 1, 3 ] [ 1, 3, 4 ]",
        "[ 1, 3 ]",
        "4",
        "1,,3",
        "1,,3",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── strict mode + private brand checks ───────────────────────────────────────

#[test]
fn strict_mode_assignment_to_an_undeclared_name_throws() {
    // 6.2.5.6 PutValue on an UNRESOLVABLE reference: strict code throws
    // ReferenceError where sloppy code creates a global. Strictness comes from a
    // directive prologue and is inherited by every nested function; a class body
    // is strict unconditionally.
    let src = r##"
        // sloppy at top level: implicit global still works
        sloppy = 1; console.log(sloppy, globalThis.sloppy);
        function f(){ 'use strict'; try { nope1 = 1 } catch(e){ console.log(e.constructor.name, e.message) } }
        f();
        function g(){ inner = 2; return inner } console.log(g());
        class C { m(){ try { nope2 = 1 } catch(e){ return e.constructor.name + ' ' + e.message } } }
        console.log(new C().m());
        (function(){ 'use strict'; function h(){ try { nope3 = 1 } catch(e){ console.log(e.constructor.name) } } h(); })();
        (function(){ 'use strict'; console = console; let x; x = 5; console.log('ok', x); })();
        (function(){ 'use strict'; try { undefined = 1 } catch(e) { console.log('undef:', e.constructor.name) } })();
        (function(){ 'use strict'; globalThis.made = 3; made = 4; console.log('made', made); })();
        console.log(typeof (function(){ 'use strict'; return this })());
    "##;
    let expected = [
        "1 1",
        "ReferenceError nope1 is not defined",
        "2",
        "ReferenceError nope2 is not defined",
        "ReferenceError",
        "ok 5",
        "undef: TypeError",
        "made 4",
        "undefined",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn readonly_globals_reject_assignment() {
    // `undefined`/`NaN`/`Infinity` are non-writable global properties, so a
    // sloppy assignment is DISCARDED rather than rebinding the name.
    let src = r##"
        undefined=1; console.log(undefined); NaN=2; console.log(NaN); Infinity=3; console.log(Infinity);
    "##;
    let expected = ["undefined", "NaN", "Infinity"].join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn private_brand_check_throws_on_a_foreign_receiver() {
    // A private element read/written/called on an object whose class never
    // declared it is a TypeError, not `undefined`. A private METHOD names the
    // class (from the running method's home class, so two classes sharing a
    // private name stay exact); a private FIELD names the member.
    let src = r##"
        class A { #v=1; static #sf=9; #pm(){return 2} get #g(){return 3} set #g(x){this.#gv=x} #gv=0;
          read(){return [this.#v, this.#pm(), this.#g]} write(x){this.#g=x; return this.#gv}
          static rs(){return A.#sf} static ws(v){A.#sf=v; return A.#sf} }
        const a=new A();
        console.log(a.read(), a.write(4), A.rs(), A.ws(11));
        console.log(Object.keys(a), Object.getOwnPropertyNames(a), JSON.stringify(a));
        console.log(Object.keys(A), Object.getOwnPropertyNames(A).filter(n=>!['length','name','prototype'].includes(n)));
        class B extends A { #b=1; useb(){return this.#b} }
        console.log(new B().useb(), new B().read());
        const t=(f)=>{try{f()}catch(e){console.log(e.constructor.name+": "+e.message)}};
        t(()=>A.prototype.read.call({}));
        t(()=>A.prototype.write.call({},1));
        t(()=>A.rs.call(null));
        class D { #dup(){return 1} u(){return this.#dup()} }
        class E { #dup(){return 2} u(){return this.#dup()} }
        console.log(new D().u(), new E().u());
        t(()=>D.prototype.u.call({}));
        console.log(a instanceof A);
    "##;
    let expected = [
        "[ 1, 2, 3 ] 4 9 11",
        "[] [] {}",
        "[] [ 'rs', 'ws' ]",
        "1 [ 1, 2, 3 ]",
        "TypeError: Cannot read private member #v from an object whose class did not declare it",
        "TypeError: Receiver must be an instance of class A",
        "1 2",
        "TypeError: Receiver must be an instance of class D",
        "true",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── fetch + generic Array.prototype ──────────────────────────────────────────

#[test]
fn fetch_round_trips_a_real_http_server() {
    // `fetch` drives the same HTTP/1.1 exchange `http.request` does, on a
    // background thread, and settles a Promise with a fully buffered `Response`.
    // Served by node-js's own `http.createServer` so the test needs no network.
    let src = r##"
        const http = require('http');
        const srv = http.createServer((req, res) => {
          let body = '';
          req.on('data', c => body += c);
          req.on('end', () => {
            if (req.url === '/json') {
              res.writeHead(200, {'Content-Type': 'application/json', 'X-Dup': 'a'});
              res.end(JSON.stringify({ok: true, method: req.method, body}));
            } else if (req.url === '/404') {
              res.writeHead(404, {'Content-Type': 'text/plain'});
              res.end('nope');
            } else {
              res.writeHead(200, {'Content-Type': 'text/plain'});
              res.end('hello ' + req.method);
            }
          });
        });
        srv.listen(0, async () => {
          const port = srv.address().port;
          const base = 'http://127.0.0.1:' + port;
          const r1 = await fetch(base + '/');
          console.log(r1.status, r1.ok, r1.statusText, await r1.text());
          const r2 = await fetch(base + '/json');
          console.log(r2.status, r2.headers.get('content-type'), JSON.stringify(await r2.json()));
          const r3 = await fetch(base + '/404');
          console.log(r3.status, r3.ok, await r3.text());
          const r4 = await fetch(base + '/json', {method: 'POST', body: 'abc', headers: {'X-A': '1'}});
          console.log(JSON.stringify(await r4.json()));
          const r5 = await fetch(base + '/');
          const buf = await r5.arrayBuffer();
          console.log(buf.byteLength);
          const r6 = await fetch(base + '/');
          console.log(Array.from(await r6.bytes()).slice(0,5).join(','));
          console.log(typeof r1.headers.get('nope'), r1.headers.has('content-type'));
          srv.close();
        });
    "##;
    let expected = [
        "200 true OK hello GET",
        "200 application/json {\"ok\":true,\"method\":\"GET\",\"body\":\"\"}",
        "404 false nope",
        "{\"ok\":true,\"method\":\"POST\",\"body\":\"abc\"}",
        "9",
        "104,101,108,108,111",
        "object true",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn fetch_classes_match_the_whatwg_shapes() {
    // `Headers` (case-insensitive, combined values, sorted iteration),
    // `Response`/`Request` bodies, `Blob`, `FormData` and `AbortController`.
    let src = r##"
        (async () => {
        const h = new Headers({'Content-Type': 'text/plain', 'X-A': '1'});
        h.append('x-a', '2');
        console.log(h.get('content-type'), h.get('X-A'), h.has('x-b'), h.get('x-b'));
        h.set('x-a', '3'); console.log(h.get('x-a'));
        h.delete('x-a'); console.log(h.has('x-a'));
        console.log([...new Headers([['b','2'],['a','1']]).entries()]);
        const r = new Response('hi', {status: 201, statusText: 'Created', headers: {'X-T':'v'}});
        console.log(r.status, r.ok, r.statusText, r.headers.get('x-t'), await r.text());
        const rj = Response.json({a:1});
        console.log(rj.status, rj.headers.get('content-type'), await rj.text());
        const req = new Request('http://x/y', {method:'post', headers:{'a':'b'}, body:'q'});
        console.log(req.url, req.method, req.headers.get('a'), await req.text());
        const b = new Blob(['ab', 'cd'], {type: 'text/plain'});
        console.log(b.size, b.type, await b.text(), await b.slice(1,3).text());
        const fd = new FormData();
        fd.append('a','1'); fd.append('a','2'); fd.set('b','3');
        console.log(fd.get('a'), fd.getAll('a'), fd.has('b'), [...fd.entries()]);
        const ac = new AbortController();
        console.log(ac.signal.aborted);
        ac.signal.addEventListener('abort', () => console.log('aborted!'));
        ac.abort();
        console.log(ac.signal.aborted, ac.signal.reason.name);
        try { ac.signal.throwIfAborted() } catch (e) { console.log('threw', e.name) }
        const s = AbortSignal.abort('why');
        console.log(s.aborted, s.reason);
        console.log(typeof AbortSignal.timeout(5));
        })();
    "##;
    let expected = [
        "text/plain 1, 2 false null",
        "3",
        "false",
        "[ [ 'a', '1' ], [ 'b', '2' ] ]",
        "201 true Created v hi",
        "200 application/json {\"a\":1}",
        "http://x/y POST b q",
        "4 text/plain abcd bc",
        "1 [ '1', '2' ] true [ [ 'a', '1' ], [ 'a', '2' ], [ 'b', '3' ] ]",
        "false",
        "aborted!",
        "true AbortError",
        "threw AbortError",
        "true why",
        "object",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn fetch_failures_are_typeerror_with_a_cause() {
    // Every transport failure is `TypeError: fetch failed` with the detail on
    // `.cause`; an aborted signal rejects with the DOMException-shaped
    // `AbortError` reason.
    let src = r##"
        const ac = new AbortController(); ac.abort();
        const r = ac.signal.reason;
        console.log(r.name, r.message, r instanceof Error, String(r));
        (async () => {
          try { await fetch('http://127.0.0.1:1/x') } catch (e) { console.log(e.constructor.name, e.message, typeof e.cause) }
          try { await fetch('nope://x') } catch (e) { console.log(e.constructor.name, e.message) }
          const c2 = new AbortController(); c2.abort();
          try { await fetch('http://127.0.0.1:1/x', {signal: c2.signal}) } catch (e) { console.log(e.name, e.message) }
        })();
    "##;
    let expected = [
        "AbortError This operation was aborted true AbortError: This operation was aborted",
        "TypeError fetch failed object",
        "TypeError fetch failed",
        "AbortError This operation was aborted",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_prototype_methods_are_generic_over_this() {
    // 23.1.3: every `Array.prototype` method is defined over
    // `LengthOfArrayLike(O)` and `Get(O, k)`, so it runs on an array-LIKE —
    // which is what makes `Array.prototype.slice.call(arguments)` work. A
    // missing index is a hole, a string receiver is dense, and a mutating method
    // writes back.
    let src = r##"
        function f(){ return Array.prototype.slice.call(arguments) }
        console.log(f(1,2,3));
        const al = {0:'a', 2:'c', length:3};
        console.log(Array.prototype.map.call(al, (x,i,o)=>[x,i,o===al]));
        console.log(Array.prototype.forEach.call(al, (x,i)=>console.log('e',i,x)));
        console.log(Object.keys(Array.prototype.slice.call(al)));
        const m = {0:3,1:1,2:2,length:3};
        console.log(Array.prototype.sort.call(m), m);
        const pu = {length:0};
        Array.prototype.push.call(pu, 'x', 'y'); console.log(pu.length, pu[0], pu[1]);
        console.log(Array.prototype.reduce.call({0:1,1:2,length:2}, (a,b)=>a+b));
        console.log(Array.prototype.concat.call([1], 2));
        console.log(Array.prototype.join.call('abc', '-'));
        console.log(Array.prototype.filter.call(al, ()=>true));
        const arr2=[1,2,3]; console.log(arr2.slice.call([9,8], 1));
        console.log([].map.call('abc', c=>c.toUpperCase()));
        try { ({}).slice() } catch(e) { console.log(e.constructor.name) }
        console.log(Array.prototype.indexOf.call({0:'z',length:1}, 'z'));
    "##;
    let expected = [
        "[ 1, 2, 3 ]",
        "[ [ 'a', 0, true ], <1 empty item>, [ 'c', 2, true ] ]",
        "e 0 a",
        "e 2 c",
        "undefined",
        "[ '0', '2' ]",
        "{ '0': 1, '1': 2, '2': 3, length: 3 } { '0': 1, '1': 2, '2': 3, length: 3 }",
        "2 x y",
        "3",
        "[ 1, 2 ]",
        "a-b-c",
        "[ 'a', 'c' ]",
        "[ 8 ]",
        "[ 'A', 'B', 'C' ]",
        "TypeError",
        "0",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn define_property_on_an_array_index_writes_the_element() {
    // `Object.defineProperty(arr, i, {value})` wrote into a property map an
    // Array does not have, so it was a silent no-op; defining past the end grows
    // the array with holes.
    let src = r##"
        const a=[1,2,3];
        Object.defineProperty(a, 1, {value: 9, writable:true, enumerable:true, configurable:true});
        console.log(a, a.length);
        const b=[1,2,3];
        Object.defineProperty(b, 5, {value: 7, writable:true, enumerable:true, configurable:true});
        console.log(b, b.length, Object.keys(b));
    "##;
    let expected = [
        "[ 1, 9, 3 ] 3",
        "[ 1, 2, 3, <2 empty items>, 7 ] 6 [ '0', '1', '2', '5' ]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── rotated loop lowering ────────────────────────────────────────────────────

#[test]
fn rotated_loops_keep_their_evaluation_order_and_count() {
    // Loops are lowered ROTATED (the test as an entry guard plus a conditional
    // backward branch at the bottom) so fusevm's tracing JIT can close a trace
    // on them. This pins that the rotation is invisible: the test still runs n+1
    // times, `continue` still re-tests, per-iteration `let` capture, labeled
    // break/continue, try/finally, generators and `yield` in the condition all
    // behave as before.
    let src = r##"
        // evaluation count of a side-effecting condition
        let n = 0; let i = 0;
        while ((n++, i < 3)) { i++; }
        console.log('while tests:', n, 'i:', i);
        let m = 0;
        for (let k = 0; (m++, k < 3); k++) {}
        console.log('for tests:', m);
        // continue re-tests
        let c = 0, t = 0;
        while ((t++, c < 5)) { c++; if (c % 2) continue; }
        console.log(c, t);
        let c2 = 0, t2 = 0, seen = [];
        for (let k = 0; (t2++, k < 5); k++) { if (k % 2) continue; seen.push(k); c2++; }
        console.log(c2, t2, seen);
        // zero iterations
        let z = 0; while (false) { z++ } for (let k = 0; false; k++) { z++ } console.log('z', z);
        // labeled break/continue
        outer: for (let a = 0; a < 3; a++) { for (let b = 0; b < 3; b++) { if (b === 1) continue outer; if (a === 2) break outer; console.log('ab', a, b); } }
        lbl: while (true) { let q = 0; while (true) { q++; if (q > 2) break lbl; } }
        console.log('lbl done');
        // per-iteration let capture
        const fns = []; for (let k = 0; k < 3; k++) fns.push(() => k);
        console.log(fns.map(f => f()));
        const fns2 = []; for (var v = 0; v < 3; v++) fns2.push(() => v);
        console.log(fns2.map(f => f()));
        // try/finally inside a loop
        let f1 = []; for (let k = 0; k < 3; k++) { try { if (k === 1) continue; f1.push(k) } finally { f1.push('f' + k) } }
        console.log(f1);
        function g() { for (let k = 0; k < 5; k++) { try { if (k === 2) return 'ret' + k } finally { } } return 'no' }
        console.log(g());
        // switch inside a loop
        let sw = []; for (let k = 0; k < 4; k++) { switch (k) { case 1: sw.push('one'); break; case 2: continue; default: sw.push(k) } }
        console.log(sw);
        // for(;;) with break
        let inf = 0; for (;;) { inf++; if (inf > 3) break } console.log('inf', inf);
        // while with break in body
        let w = 0; while (true) { w++; if (w === 4) break } console.log('w', w);
        // nested while + do-while
        let acc = []; let x = 0; while (x < 2) { let y = 0; do { acc.push([x,y]); y++ } while (y < 2); x++ } console.log(acc.length);
        // condition mutating the loop variable
        let mv = 0, cnt = 0; while (mv++ < 3) { cnt++ } console.log(mv, cnt);
        // generator with a loop
        function* gen() { let k = 0; while (k < 3) { yield k; k++ } }
        console.log([...gen()]);
        function* gen2() { for (let k = 0; k < 3; k++) yield k }
        console.log([...gen2()]);
        // yield in the condition
        function* gen3() { let k = 0; while (yield k) { k++ } return k }
        const it = gen3(); console.log(it.next().value, it.next(true).value, it.next(false));
        // async loop
        (async () => { let s = 0; for (let k = 0; k < 3; k++) { s += await Promise.resolve(k) } console.log('async', s) })();
    "##;
    let expected = [
        "while tests: 4 i: 3",
        "for tests: 4",
        "5 6",
        "3 6 [ 0, 2, 4 ]",
        "z 0",
        "ab 0 0",
        "ab 1 0",
        "lbl done",
        "[ 0, 1, 2 ]",
        "[ 3, 3, 3 ]",
        "[ 0, 'f0', 'f1', 2, 'f2' ]",
        "ret2",
        "[ 0, 'one', 3 ]",
        "inf 4",
        "w 4",
        "4",
        "4 3",
        "[ 0, 1, 2 ]",
        "[ 0, 1, 2 ]",
        "0 1 { value: 1, done: true }",
        "async 3",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── the module resolution cache ──────────────────────────────────────────────

/// A repeated `require` of an already-loaded module resolves through a memoized
/// `(specifier, from_dir)` table instead of walking the filesystem again. This
/// pins that the memo cannot change what resolves: the same file reached by
/// three different specifiers is still ONE module instance with one shared
/// closure state, a re-require still returns the identical exports object, and a
/// specifier that does not resolve still fails the same way the second time.
///
/// Expectations captured from node v26.7.0 (its `Module._pathCache` is the same
/// memo, with the same consequence).
#[test]
fn module_path_memoization_does_not_change_what_resolves() {
    let dir = tempfile::tempdir().expect("temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir sub");
    std::fs::write(
        dir.path().join("a.js"),
        "let n = 0;\nmodule.exports = { bump: () => ++n, get: () => n };\n",
    )
    .expect("write a.js");
    std::fs::write(
        sub.join("b.js"),
        "const a1 = require('../a.js');\n\
         const a2 = require('../a');\n\
         const a3 = require(require('path').resolve(__dirname, '..', 'a.js'));\n\
         a1.bump(); a2.bump(); a3.bump();\n\
         module.exports = { same: a1 === a2 && a2 === a3, n: a1.get() };\n",
    )
    .expect("write b.js");
    let main = dir.path().join("main.js");
    std::fs::write(
        &main,
        "const b = require('./sub/b.js');\n\
         console.log(b.same, b.n);\n\
         const a = require('./a.js');\n\
         console.log(a.get(), require('./a.js') === a, require('./a') === a);\n\
         let t = 0; for (let i = 0; i < 500; i++) t += require('./a.js').get();\n\
         console.log(t);\n\
         try { require('./nope.js') } catch (e) { console.log(e.code) }\n\
         try { require('./nope.js') } catch (e) { console.log(e.code) }\n",
    )
    .expect("write main.js");

    let out = run_bounded_out(&main);
    assert!(
        out.status.success(),
        "program failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        [
            "true 3",
            "3 true true",
            "1500",
            "MODULE_NOT_FOUND",
            "MODULE_NOT_FOUND"
        ]
        .join("\n")
    );
}

// ── accessors, class statics, iterator closing, and the ES sweep ─────────────

#[test]
fn own_accessors_enumerate_and_render_where_declared() {
    // An own accessor takes its slot in insertion order among the data
    // properties (an object literal reserves it at compile time, so `{ get
    // g(){}, d: 2 }` enumerates `g, d`), and `util.inspect` shows it as
    // `[Getter]`/`[Setter]`/`[Getter/Setter]` rather than omitting it.
    let src = r##"
        console.log(Object.getOwnPropertyDescriptors({get g(){return 1}, d:2}));
        const o = {a:1, get b(){return 2}, c:3, set b(v){}, get e(){return 5}};
        console.log(Object.keys(o), JSON.stringify(o), o.b, o.e);
        const k='cc';
        const o2 = {x:1, get [k](){return 9}, y:2};
        console.log(Object.keys(o2), o2.cc);
        const o3 = {get a(){return 1}, set a(v){this._v=v}};
        console.log(Object.keys(o3), Object.getOwnPropertyDescriptor(o3,'a').get !== undefined, Object.getOwnPropertyDescriptor(o3,'a').set !== undefined);
        console.log({get z(){return 1}});
        console.log(Object.entries({get p(){return 7}, q:8}));
        console.log({...{get w(){return 3}, v:4}});
    "##;
    let expected = [
        "{",
        "  g: {",
        "    get: [Function: get g],",
        "    set: undefined,",
        "    enumerable: true,",
        "    configurable: true",
        "  },",
        "  d: { value: 2, writable: true, enumerable: true, configurable: true }",
        "}",
        "[ 'a', 'b', 'c', 'e' ] {\"a\":1,\"b\":2,\"c\":3,\"e\":5} 2 5",
        "[ 'x', 'cc', 'y' ] 9",
        "[ 'a' ] true true",
        "{ z: [Getter] }",
        "[ [ 'p', 7 ], [ 'q', 8 ] ]",
        "{ w: 3, v: 4 }",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn non_enumerable_and_inherited_accessors_stay_hidden() {
    // Only an own ENUMERABLE accessor renders; a non-enumerable one and a class
    // prototype's getter do not.
    let src = r##"
        const o = {}; Object.defineProperty(o,'ng',{get(){return 1}});
        console.log(o, Object.keys(o));
        Object.defineProperty(o,'eg',{get(){return 1}, enumerable:true});
        console.log(o);
        class C { get p(){return 1} }
        console.log(new C(), Object.keys(new C()));
        const inst = new C(); Object.defineProperty(inst,'own',{get(){return 2},enumerable:true});
        console.log(inst);
        console.log({a:{get b(){return 1}}});
        console.log([{get c(){return 1}}]);
        console.log(JSON.stringify({get j(){return 5}, k:6}));
        console.log(Object.assign({}, {get m(){return 7}}));
        const s=Symbol('sy'); const o4={[s]:1, get n(){return 2}}; console.log(o4);
        console.log(util_check());
        function util_check(){ return require('util').inspect({get q(){return 1}}) }
    "##;
    let expected = [
        "{} []",
        "{ eg: [Getter] }",
        "C {} []",
        "C { own: [Getter] }",
        "{ a: { b: [Getter] } }",
        "[ { c: [Getter] } ]",
        "{\"j\":5,\"k\":6}",
        "{ m: 7 }",
        "{ n: [Getter], Symbol(sy): 1 }",
        "{ q: [Getter] }",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn class_statics_enumerate_in_class_evaluation_order() {
    // ClassDefinitionEvaluation installs methods and accessors while evaluating
    // the body and only then runs the static-field initializers, so
    // `getOwnPropertyNames` lists a static getter before a static field declared
    // ahead of it.
    let src = r##"
        class A { static s = 2; static get sv(){return 1} static m(){} }
        console.log(Object.getOwnPropertyNames(A).filter(n=>!['length','name','prototype'].includes(n)));
        const o = {}; Object.defineProperty(o,'g',{get(){return 1},enumerable:true,configurable:true}); o.d = 2;
        console.log(Object.keys(o), Object.getOwnPropertyNames(o));
        const o2 = {}; o2.d = 1; Object.defineProperty(o2,'g',{get(){return 1},enumerable:true,configurable:true}); o2.e = 3;
        console.log(Object.keys(o2));
        class B { static get g(){return 1} static f = 2 }
        console.log(Object.getOwnPropertyNames(B).filter(n=>!['length','name','prototype'].includes(n)));
    "##;
    let expected = [
        "[ 'sv', 'm', 's' ]",
        "[ 'g', 'd' ] [ 'g', 'd' ]",
        "[ 'd', 'g', 'e' ]",
        "[ 'g', 'f' ]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn generators_descriptors_weak_collections_and_error_cause() {
    // yield* delegation, async generators and `for await`, try/finally
    // overriding a return, labeled break, tagged templates and String.raw, error
    // `cause`, property descriptors, and Map/Set semantics — a weak collection
    // never shows its contents and `-0` is stored as `+0`.
    let src = r##"
        function* inner(){ yield 1; yield 2; return 'ir' }
        function* outer(){ const r = yield* inner(); console.log('got', r); yield 3 }
        const g = outer();
        console.log(g.next(), g.next(), g.next(), g.next());
        function* g4(){ try { yield 1 } catch(e) { yield 'c:'+e.message } }
        const i4 = g4(); i4.next(); console.log(i4.throw(new Error('x')));
        async function* ag(){ yield 1; yield await Promise.resolve(2); yield 3 }
        (async () => {
          const out = []; for await (const v of ag()) out.push(v); console.log('ag', out);
          for await (const v of [Promise.resolve('a'), 'b']) console.log('fa', v);
          const it = ag(); console.log(await it.next(), await it.return('r'));
        })();
        function f1(){ try { return 1 } finally { return 2 } }
        console.log(f1());
        function f3(){ lbl: { console.log('in'); break lbl; } return 'done' }
        console.log(f3());
        function tag(s, ...v){ return [s.raw.join('|'), s.join('|'), v.join(',')] }
        console.log(tag`a${1}b\n${2}c`);
        console.log(String.raw`x\ny${1+1}z`, String.raw({raw:['a','b','c']}, 1, 2));
        const e1 = new Error('outer', { cause: new Error('inner') });
        console.log(e1.message, e1.cause.message, 'cause' in e1, Object.keys(e1));
        console.log(new TypeError('t', { cause: 42 }).cause, new Error('x').cause);
        const o = {}; Object.defineProperty(o, 'a', {value:1});
        console.log(Object.getOwnPropertyDescriptor(o,'a'), Object.keys(o), o.a);
        const m = new Map([['a',1],[NaN,2]]); m.set(-0, 3);
        console.log(m, m.get(NaN), m.get(0), m.size, [...m.keys()]);
        const s = new Set([1,1,NaN,NaN,-0,0]); console.log(s, s.size, s.has(NaN));
        const wm = new WeakMap(); const k = {}; wm.set(k, 1); console.log(wm.get(k), wm.has({}), wm);
        const ws = new WeakSet(); ws.add(k); console.log(ws.has(k), ws);
        console.log(new Map([[{a:1}, [1,2]]]), new Set([[1,2],{a:1}]));
    "##;
    let expected = [
        "got ir",
        "{ value: 1, done: false } { value: 2, done: false } { value: 3, done: false } { value: undefined, done: true }",
        "{ value: 'c:x', done: false }",
        "2",
        "in",
        "done",
        "[ 'a|b\\\\n|c', 'a|b\\n|c', '1,2' ]",
        "x\\ny2z a1b2c",
        "outer inner true []",
        "42 undefined",
        "{ value: 1, writable: false, enumerable: false, configurable: false } [] 1",
        "Map(3) { 'a' => 1, NaN => 2, 0 => 3 } 2 3 3 [ 'a', NaN, 0 ]",
        "Set(3) { 1, NaN, 0 } 3 true",
        "1 false WeakMap { <items unknown> }",
        "true WeakSet { <items unknown> }",
        "Map(1) { { a: 1 } => [ 1, 2 ] } Set(2) { [ 1, 2 ], { a: 1 } }",
        "ag [ 1, 2, 3 ]",
        "fa a",
        "fa b",
        "{ value: 1, done: false } { value: 'r', done: true }",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn class_fields_proxy_reflect_and_well_known_symbols() {
    // Public/private/static class fields, static blocks, getters and setters;
    // Proxy traps and Reflect; `Symbol.hasInstance`, `Symbol.toPrimitive`,
    // `Symbol.toStringTag` and `Symbol.iterator`.
    let src = r##"
        // class fields, getters/setters, static blocks
        class A { x = 1; static s = 2; #p = 3; static { A.fromBlock = 9 } get v(){return this.x} set v(n){this.x=n} static get sv(){return A.s} }
        const a = new A(); a.v = 5;
        console.log(a.x, A.s, A.fromBlock, a.v, A.sv, Object.keys(a), Object.getOwnPropertyNames(A).filter(n=>!['length','name','prototype'].includes(n)));
        console.log(Object.getOwnPropertyDescriptor(A.prototype,'v'));
        // Proxy / Reflect
        const t = {a:1};
        const p = new Proxy(t, { get:(o,k,r)=> k==='b'?42:Reflect.get(o,k,r), has:(o,k)=>k==='z'||k in o, ownKeys:o=>[...Reflect.ownKeys(o),'v'], getOwnPropertyDescriptor:(o,k)=>k==='v'?{value:7,enumerable:true,configurable:true}:Reflect.getOwnPropertyDescriptor(o,k), set:(o,k,v)=>Reflect.set(o,k,v), deleteProperty:(o,k)=>Reflect.deleteProperty(o,k) });
        console.log(p.a, p.b, 'z' in p, Object.keys(p), JSON.stringify(p));
        p.c = 3; console.log(t.c, delete p.c, t.c);
        const { proxy, revoke } = Proxy.revocable({}, {}); revoke();
        try { proxy.x } catch(e) { console.log(e.constructor.name) }
        console.log(Reflect.ownKeys({a:1,[Symbol('s')]:2}).length, Reflect.has({a:1},'a'), Reflect.apply(Math.max,null,[1,2,3]));
        // well-known symbols
        class B { static [Symbol.hasInstance](x){ return x === 1 } }
        console.log(1 instanceof B, 2 instanceof B);
        const c = { [Symbol.toPrimitive](h){ return h==='number'?10:'str' } };
        console.log(+c, `${c}`, c+'');
        class D { get [Symbol.toStringTag](){ return 'Dee' } }
        console.log(Object.prototype.toString.call(new D()), String(new D()));
        const it = { *[Symbol.iterator](){ yield 1; yield 2 } };
        console.log([...it]);
    "##;
    let expected = [
        "5 2 9 5 2 [ 'x' ] [ 'sv', 's', 'fromBlock' ]",
        "{",
        "  get: [Function: get v],",
        "  set: [Function: set v],",
        "  enumerable: false,",
        "  configurable: true",
        "}",
        "1 42 true [ 'a', 'v' ] {\"a\":1}",
        "3 true undefined",
        "TypeError",
        "2 true 3",
        "true false",
        "10 str str",
        "[object Dee] [object Dee]",
        "[ 1, 2 ]",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn an_abrupt_return_closes_every_open_for_of_iterator() {
    // 7.4.9 IteratorClose: a `return` out of a `for…of` closes the iterator,
    // which is what runs a generator's pending `finally`. `break` and
    // destructuring already did; a `return` walked away and left the generator
    // suspended. Every iterator the chunk has parked is closed, innermost first,
    // with the return value kept on the stack.
    let src = r##"
        function* inner(){ try { yield 1; yield 2 } finally { console.log('inner finally') } }
        function f(){ for (const v of inner()) return 'ret'+v }
        console.log(f());
        function g(){ for (const v of inner()) break; return 'brk' }
        console.log(g());
        const [x] = inner(); console.log('destructure', x);
        for (const v of inner()) { console.log('of', v); break }
        function a(){ for (const v of inner()) { for (const w of inner()) return [v,w] } }
        console.log(a());
        function b(){ try { for (const v of inner()) return v } finally { console.log('outer fin') } }
        console.log(b());
        function c(){ for (const v of inner()) { if (v===1) continue; return v } return 'none' }
        console.log(c());
        function e(){ for (const v of [1,2,3]) return v }
        console.log(e());
        async function h(){ for (const v of inner()) return v }
        h().then(v=>console.log('async', v));
    "##;
    let expected = [
        "inner finally",
        "ret1",
        "inner finally",
        "brk",
        "inner finally",
        "destructure 1",
        "of 1",
        "inner finally",
        "inner finally",
        "inner finally",
        "[ 1, 1 ]",
        "inner finally",
        "outer fin",
        "1",
        "inner finally",
        "2",
        "1",
        "inner finally",
        "async 1",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── callee text in a failed call's TypeError ─────────────────────────────────

#[test]
fn a_failed_call_names_the_callee_as_the_source_wrote_it() {
    // V8 reports the callee by re-printing its AST — `z.f is not a function`,
    // not `f is not a function` — so the text is a static property of the call
    // SITE and the compiler records it once per call op. A string-literal
    // computed key normalizes to dot form (`o['a']` prints `o.a`), a call in a
    // callee position prints as `f(...)`, and optional chaining is kept.
    let src = r##"
        const o={a:{b:{}}}, k='f', z={};
        const t=(f)=>{try{f()}catch(e){console.log(e.message)}};
        t(()=>z.f());
        t(()=>o.a.b.c());
        t(()=>o[k]());
        t(()=>o['a']['zz']());
        t(()=>o.a.b['x']());
        t(()=>({}).x());
        t(()=>[].x());
        t(()=>"s".x());
        t(()=>(3).x());
        t(()=>Math.nope());
        t(()=>JSON.nope());
        t(()=>o?.a?.zz());
        t(()=>{const q=1; q()});
        t(()=>{let f; f()});
        const arr=[1]; t(()=>arr.nope());
    "##;
    let expected = [
        "z.f is not a function",
        "o.a.b.c is not a function",
        "o[k] is not a function",
        "o.a.zz is not a function",
        "o.a.b.x is not a function",
        "{}.x is not a function",
        "[].x is not a function",
        "\"s\".x is not a function",
        "3.x is not a function",
        "Math.nope is not a function",
        "JSON.nope is not a function",
        "o?.a?.zz is not a function",
        "q is not a function",
        "f is not a function",
        "arr.nope is not a function",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn an_untested_for_keeps_its_semantics() {
    // `for (;;)` closes with a constant-true CONDITIONAL branch so the tracing
    // JIT can compile it. This pins that the shape change is invisible: break,
    // continue, labeled break/continue, return, a generator, a throw out of the
    // loop, and try/finally all behave as before.
    let src = r##"
        let a=0; for(;;){ a++; if(a>3) break } console.log('a',a);
        let b=0,c=0; for(;;){ b++; if(b>5) break; if(b%2) continue; c++ } console.log(b,c);
        outer: for(;;){ for(;;){ break outer } } console.log('lbl ok');
        let d=0; outer2: for(;;){ d++; if(d>2) break outer2; continue outer2 } console.log('d',d);
        function f(){ for(;;){ return 'r' } } console.log(f());
        function* g(){ let i=0; for(;;){ yield i++; if(i>2) return 'done' } }
        console.log([...g()]);
        let e=0; try { for(;;){ e++; if(e>2) throw new Error('x') } } catch(err){ console.log('caught',e) }
        let h=0; for(;;){ try { h++; if(h>2) break } finally { if(h>2) console.log('fin') } } console.log('h',h);
        let k=0; for(;;) { k++; if (k>1) break } console.log('k',k);
    "##;
    let expected = [
        "a 4",
        "6 2",
        "lbl ok",
        "d 3",
        "r",
        "[ 0, 1, 2 ]",
        "caught 3",
        "fin",
        "h 3",
        "k 2",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── require.main ─────────────────────────────────────────────────────────────

/// `require.main === module` is the canonical "am I the program" test, and it
/// read `undefined === <module>` because nothing ever set `main`. Every
/// `require` in the process reports the same value — the ENTRY module — and a
/// program that is not a module at all (`node -e`, a script on stdin) reports
/// `undefined`, as node does.
#[test]
fn require_main_is_the_entry_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("lib.js"),
        "module.exports = { isMain: require.main === module, mainId: require.main && require.main.id };\n",
    )
    .expect("write lib.js");
    let app = dir.path().join("app.js");
    std::fs::write(
        &app,
        "console.log('entry isMain:', require.main === module);\n\
         console.log('entry mainIsEntry:', require.main.filename === module.filename);\n\
         const lib = require('./lib.js');\n\
         console.log('lib isMain:', lib.isMain, 'sameMain:', lib.mainId === require.main.id);\n\
         console.log(typeof require.main, typeof require.resolve);\n",
    )
    .expect("write app.js");

    let out = run_bounded_out(&app);
    assert!(
        out.status.success(),
        "program failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        [
            "entry isMain: true",
            "entry mainIsEntry: true",
            "lib isMain: false sameMain: true",
            "object function",
        ]
        .join("\n")
    );

    // `node -e` runs as a Script, not a module: no main.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_node"))
        .args(["-e", "console.log(typeof require.main)"])
        .output()
        .expect("spawn node binary");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), "undefined");
}

// ── 64-bit typed-array views ─────────────────────────────────────────────────

#[test]
fn bigint64_views_keep_full_64_bit_precision() {
    // `BigInt64Array`/`BigUint64Array` store BigInt elements, so a value beyond
    // 2^53 survives a round trip where an `f64` element would round it. Writing
    // a Number into one is a TypeError, wrapping is modulo 2^64 into the view's
    // signedness, and `sort` orders the integers rather than their nearest
    // doubles.
    let src = r##"
        for (const K of [BigInt64Array, BigUint64Array]) {
          const a = new K([1n, 2n, 3n]);
          console.log(K.name, a, a.length, a.byteLength, a.BYTES_PER_ELEMENT);
          console.log(' idx', a[0], typeof a[0], a.at(-1), a.indexOf(2n), a.includes(3n), a.includes(3));
          console.log(' join', a.join('-'), a.toString(), [...a]);
          console.log(' map', a.map(x=>x*2n));
          console.log(' filter', a.filter(x=>x>1n));
          console.log(' reduce', a.reduce((p,q)=>p+q, 0n));
          console.log(' sort', new K([3n,1n,2n]).sort());
          console.log(' reverse', new K([1n,2n,3n]).reverse());
          console.log(' slice', a.slice(1), a.subarray(0,2));
          console.log(' fill', new K(3).fill(7n), new K(2));
          const b = new K(3); b.set([9n,8n],1); console.log(' set', b);
          const c = new K(2); c[0] = 5n; console.log(' write', c);
          console.log(' keys', [...a.keys()], [...a.entries()]);
          console.log(' from/of', K.from([1n,2n]), K.of(4n,5n));
          console.log(' every', a.every(x=>x>0n), a.some(x=>x>2n), a.find(x=>x>1n), a.findIndex(x=>x>1n));
        }
        console.log('wrap', new BigInt64Array([2n**63n]), new BigUint64Array([-1n]));
        console.log('big', new BigInt64Array([2n**63n - 1n])[0], new BigUint64Array([2n**64n - 1n])[0]);
        console.log('precision', new BigInt64Array([9007199254740993n])[0]);
        try { const t = new BigInt64Array(1); t[0] = 1; } catch(e) { console.log('numwrite', e.constructor.name) }
        try { new BigInt64Array([1]) } catch(e) { console.log('numctor', e.constructor.name) }
        console.log('tag', Object.prototype.toString.call(new BigInt64Array(1)));
        console.log('isview', ArrayBuffer.isView(new BigInt64Array(1)));
    "##;
    let expected = [
        "BigInt64Array BigInt64Array(3) [ 1n, 2n, 3n ] 3 24 8",
        " idx 1n bigint 3n 1 true false",
        " join 1-2-3 1,2,3 [ 1n, 2n, 3n ]",
        " map BigInt64Array(3) [ 2n, 4n, 6n ]",
        " filter BigInt64Array(2) [ 2n, 3n ]",
        " reduce 6n",
        " sort BigInt64Array(3) [ 1n, 2n, 3n ]",
        " reverse BigInt64Array(3) [ 3n, 2n, 1n ]",
        " slice BigInt64Array(2) [ 2n, 3n ] BigInt64Array(2) [ 1n, 2n ]",
        " fill BigInt64Array(3) [ 7n, 7n, 7n ] BigInt64Array(2) [ 0n, 0n ]",
        " set BigInt64Array(3) [ 0n, 9n, 8n ]",
        " write BigInt64Array(2) [ 5n, 0n ]",
        " keys [ 0, 1, 2 ] [ [ 0, 1n ], [ 1, 2n ], [ 2, 3n ] ]",
        " from/of BigInt64Array(2) [ 1n, 2n ] BigInt64Array(2) [ 4n, 5n ]",
        " every true true 2n 1",
        "BigUint64Array BigUint64Array(3) [ 1n, 2n, 3n ] 3 24 8",
        " idx 1n bigint 3n 1 true false",
        " join 1-2-3 1,2,3 [ 1n, 2n, 3n ]",
        " map BigUint64Array(3) [ 2n, 4n, 6n ]",
        " filter BigUint64Array(2) [ 2n, 3n ]",
        " reduce 6n",
        " sort BigUint64Array(3) [ 1n, 2n, 3n ]",
        " reverse BigUint64Array(3) [ 3n, 2n, 1n ]",
        " slice BigUint64Array(2) [ 2n, 3n ] BigUint64Array(2) [ 1n, 2n ]",
        " fill BigUint64Array(3) [ 7n, 7n, 7n ] BigUint64Array(2) [ 0n, 0n ]",
        " set BigUint64Array(3) [ 0n, 9n, 8n ]",
        " write BigUint64Array(2) [ 5n, 0n ]",
        " keys [ 0, 1, 2 ] [ [ 0, 1n ], [ 1, 2n ], [ 2, 3n ] ]",
        " from/of BigUint64Array(2) [ 1n, 2n ] BigUint64Array(2) [ 4n, 5n ]",
        " every true true 2n 1",
        "wrap BigInt64Array(1) [ -9223372036854775808n ] BigUint64Array(1) [ 18446744073709551615n ]",
        "big 9223372036854775807n 18446744073709551615n",
        "precision 9007199254740993n",
        "numwrite TypeError",
        "numctor TypeError",
        "tag [object BigInt64Array]",
        "isview true",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn typed_array_search_does_not_coerce_its_argument() {
    // 23.2.3.x compare the search element with the STORED one and do not coerce
    // it, so a string never matches a numeric element; `includes` differs from
    // `indexOf` only in using SameValueZero, which is the NaN case.
    let src = r##"
        const a = new Uint8Array([1,2,3]);
        console.log(a.indexOf('2'), a.indexOf(2), a.includes('2'), a.includes(2), a.lastIndexOf('3'));
        const f = new Float64Array([NaN, 1, -0]);
        console.log(f.includes(NaN), f.indexOf(NaN), f.includes(0), f.indexOf(-0), f.indexOf(0));
        console.log(new Uint8Array([1]).indexOf(null), new Uint8Array([0]).includes(null));
    "##;
    let expected = ["-1 1 false true -1", "true -1 true 2 2", "-1 false"].join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn typed_array_index_keys_are_own_enumerable_properties() {
    // A typed array is an index-keyed exotic: its own enumerable keys are its
    // indices, so `Object.keys`, `JSON.stringify`, object spread and `for…in`
    // all see them. Only `Buffer` had that arm, so every other view enumerated
    // as empty while `hasOwnProperty(0)` already answered true.
    let src = r##"
        const b = Buffer.from([1,2,250]);
        console.log(Object.keys(b), JSON.stringify(b), {...b});
        console.log(b, b.length, b.toJSON());
        console.log(JSON.stringify({b}), Object.entries(b).length);
        const u = new Uint8Array([7,8]);
        console.log(Object.keys(u), JSON.stringify(u), {...u}, Object.entries(u));
        console.log(JSON.stringify(new Float64Array([1.5])), JSON.stringify(new BigInt64Array(0)));
        for (const k in u) console.log('in', k);
    "##;
    let expected = [
        "[ '0', '1', '2' ] {\"type\":\"Buffer\",\"data\":[1,2,250]} { '0': 1, '1': 2, '2': 250 }",
        "<Buffer 01 02 fa> 3 { type: 'Buffer', data: [ 1, 2, 250 ] }",
        "{\"b\":{\"type\":\"Buffer\",\"data\":[1,2,250]}} 3",
        "[ '0', '1' ] {\"0\":7,\"1\":8} { '0': 7, '1': 8 } [ [ '0', 7 ], [ '1', 8 ] ]",
        "{\"0\":1.5} {}",
        "in 0",
        "in 1",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── require.cache ────────────────────────────────────────────────────────────

/// `require.cache` is a LIVE view of the module cache, not a populated copy, so
/// `delete require.cache[id]` actually invalidates and the next `require` runs
/// the file again. It used to be an empty object literal: reads answered
/// `undefined` and a delete silently did nothing.
#[test]
fn require_cache_is_the_live_module_cache() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("dep.js"),
        "console.log('dep evaluated');\n\
         module.exports = { n: (globalThis.__n = (globalThis.__n || 0) + 1) };\n",
    )
    .expect("write dep.js");
    let main = dir.path().join("main.js");
    std::fs::write(
        &main,
        "const a = require('./dep.js');\n\
         const b = require('./dep.js');\n\
         console.log('cached', a === b, a.n, b.n);\n\
         const id = require.resolve('./dep.js');\n\
         console.log('has', typeof require.cache[id] === 'object', Object.keys(require.cache).length > 0);\n\
         console.log('exports match', require.cache[id].exports === a);\n\
         delete require.cache[id];\n\
         const c = require('./dep.js');\n\
         console.log('after delete', c === a, c.n);\n\
         console.log('missing', require.cache['/nope.js'], typeof require.cache);\n",
    )
    .expect("write main.js");

    let out = run_bounded_out(&main);
    assert!(
        out.status.success(),
        "program failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        [
            "dep evaluated",
            "cached true 1 1",
            "has true true",
            "exports match true",
            // the delete forced a re-run, so the module body printed again
            "dep evaluated",
            "after delete false 2",
            "missing undefined object",
        ]
        .join("\n")
    );
}

#[test]
fn an_injected_return_closes_the_parked_iterators() {
    // A `.return()`/`.throw()` injected at a suspension point halts the
    // generator's chunk, which jumps past the loop exits that would have closed
    // its `for…of` / `yield*` iterators — so they were abandoned
    // still-suspended and their `finally` never ran. The compiler records how
    // many are parked at each `yield`, and the halt path closes them
    // innermost-first, saving the pending completion across the close so it
    // survives.
    let src = r##"
        function* inner(){ try { yield 1; yield 2 } finally { console.log('  inner finally') } }
        console.log('A'); (function(){ for (const v of inner()) return v })();
        console.log('B'); (function(){ for (const v of inner()) break })();
        console.log('D'); { function* o(){ for (const v of inner()) yield v } const g=o(); g.next(); console.log(' ', g.return('r')); }
        console.log('E'); { function* o(){ yield* inner() } const g=o(); g.next(); console.log(' ', g.return('r')); }
        console.log('F'); { function* o(){ yield* inner() } const g=o(); g.next(); try{g.throw(new Error('t'))}catch(e){console.log('  caught', e.message)} }
        console.log('G'); { function* m(){ yield* inner() } function* o(){ yield* m() } const g=o(); g.next(); console.log(' ', g.return('r')); }
        console.log('H'); { const [x] = inner(); }
        console.log('J two levels'); { function* o(){ for (const a of inner()) for (const b of inner()) yield [a,b] } const g=o(); g.next(); console.log(' ', g.return('z')); }
        console.log('K delegate finishes'); { function* o(){ yield* inner(); yield 'after' } console.log(' ', [...o()]); }
        console.log('L outer finally too'); { function* o(){ try { yield* inner() } finally { console.log('  outer finally') } } const g=o(); g.next(); g.return('r'); }
    "##;
    let expected = [
        "A",
        "  inner finally",
        "B",
        "  inner finally",
        "D",
        "  inner finally",
        "  { value: 'r', done: true }",
        "E",
        "  inner finally",
        "  { value: 'r', done: true }",
        "F",
        "  inner finally",
        "  caught t",
        "G",
        "  inner finally",
        "  { value: 'r', done: true }",
        "H",
        "  inner finally",
        "J two levels",
        "  inner finally",
        "  inner finally",
        "  { value: 'z', done: true }",
        "K delegate finishes",
        "  inner finally",
        "  [ 1, 2, 'after' ]",
        "L outer finally too",
        "  inner finally",
        "  outer finally",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

// ── web-platform globals ─────────────────────────────────────────────────────

#[test]
fn web_platform_globals_match_node() {
    // URL/URLSearchParams, TextEncoder/TextDecoder, queueMicrotask ordering
    // against promise reactions, AggregateError and Promise.any, and
    // Object.groupBy/Map.groupBy.
    let src = r##"
        console.log('--- URL');
        const u = new URL('https://a:b@h.io:8443/p/q?x=1&y=2#f');
        console.log(u.href, u.protocol, u.host, u.hostname, u.port, u.pathname, u.search, u.hash, u.origin, u.username, u.password);
        console.log(String(u), JSON.stringify(u), u.toJSON());
        const sp = new URLSearchParams('a=1&b=2&a=3');
        console.log(sp.get('a'), sp.getAll('a'), sp.has('b'), sp.toString());
        sp.append('c','4'); sp.set('a','9'); sp.delete('b'); console.log(sp.toString(), sp.size);
        console.log('iterable:', typeof sp[Symbol.iterator]);
        console.log(new URL('/x','https://h.io/a/b').href, new URL('c','https://h.io/a/b').href);
        console.log('--- TextEncoder/Decoder');
        const te = new TextEncoder(); const bytes = te.encode('héllo');
        console.log(te.encoding, bytes, bytes.length, new TextDecoder().decode(bytes));
        console.log(new TextDecoder('utf-8').decode(new Uint8Array([226,130,172])));
        console.log('--- queueMicrotask ordering');
        Promise.resolve().then(()=>console.log('p1'));
        queueMicrotask(()=>console.log('q1'));
        Promise.resolve().then(()=>console.log('p2'));
        queueMicrotask(()=>console.log('q2'));
        console.log('sync');
        console.log('--- AggregateError / Promise.any');
        (async()=>{
          try { await Promise.any([Promise.reject(new Error('a')), Promise.reject(new Error('b'))]) }
          catch(e){ console.log(e.constructor.name, e.message, e.errors.map(x=>x.message)) }
          console.log(await Promise.any([Promise.reject(new Error('a')), Promise.resolve('ok')]));
          const ae = new AggregateError([new Error('x')], 'msg');
          console.log(ae.name, ae.message, ae.errors.length, ae instanceof Error);
        })();
        console.log('--- Object.groupBy / Map.groupBy');
        console.log(JSON.stringify(Object.groupBy([1,2,3,4], x=>x%2?'odd':'even')));
        console.log([...Map.groupBy([1,2,3], x=>x>1)]);
    "##;
    let expected = [
        "--- URL",
        "https://a:b@h.io:8443/p/q?x=1&y=2#f https: h.io:8443 h.io 8443 /p/q ?x=1&y=2 #f https://h.io:8443 a b",
        "https://a:b@h.io:8443/p/q?x=1&y=2#f \"https://a:b@h.io:8443/p/q?x=1&y=2#f\" https://a:b@h.io:8443/p/q?x=1&y=2#f",
        "1 [ '1', '3' ] true a=1&b=2&a=3",
        "a=9&c=4 2",
        "iterable: function",
        "https://h.io/x https://h.io/a/c",
        "--- TextEncoder/Decoder",
        "utf-8 Uint8Array(6) [ 104, 195, 169, 108, 108, 111 ] 6 héllo",
        "€",
        "--- queueMicrotask ordering",
        "sync",
        "--- AggregateError / Promise.any",
        "--- Object.groupBy / Map.groupBy",
        "{\"odd\":[1,3],\"even\":[2,4]}",
        "[ [ false, [ 1 ] ], [ true, [ 2, 3 ] ] ]",
        "p1",
        "q1",
        "p2",
        "q2",
        "AggregateError All promises were rejected [ 'a', 'b' ]",
        "ok",
        "AggregateError msg 1 true",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn native_objects_are_iterable_and_render_as_node_does() {
    // A native-tagged object dispatches its methods through the stdlib table
    // rather than a property map, so `Symbol.iterator` was invisible to the
    // iteration protocol and `[...new URLSearchParams('a=1')]` threw. Also
    // covers toLocaleString fallbacks, freeze with accessors, and
    // structuredClone of Map/Set/Date/holes/cycles.
    let src = r##"
        const sp = new URLSearchParams('a=1&b=2');
        console.log(sp, Object.keys(sp), JSON.stringify(sp), sp.size);
        sp.append('c','3'); console.log(sp.size, [...sp]);
        sp.delete('a'); console.log(sp.size, sp.toString());
        console.log([...new URL('https://h.io/?x=1').searchParams], new URL('https://h.io/?x=1&y=2').searchParams.size);
        console.log('--- iterate other natives');
        console.log([...new Headers({a:'1',b:'2'})]);
        const fd = new FormData(); fd.append('k','v'); console.log([...fd]);
        console.log([...new Map([[1,2]])], [...new Set([1])], [...'ab'], [...new Uint8Array([1,2])]);
        console.log([...new BigInt64Array([1n])]);
        console.log('--- toLocaleString fallbacks');
        console.log((1234.5678).toLocaleString(), (0).toLocaleString(), (-1234).toLocaleString());
        console.log([1,'a',true].toLocaleString(), new Date(0).toLocaleString().length > 0);
        console.log('--- freeze + accessors');
        const o = { get g(){return 1}, d: 2 }; Object.freeze(o);
        console.log(Object.isFrozen(o), Object.getOwnPropertyDescriptor(o,'g').configurable, o.g);
        o.d = 9; console.log(o.d);
        const f2 = Object.freeze({a:1}); try { 'use strict'; f2.a = 2 } catch(e) { console.log('frozen write', e.constructor.name) }
        console.log('--- structuredClone extras');
        console.log(structuredClone(new Map([[1,{a:2}]])), structuredClone(new Set([1,2])));
        console.log(structuredClone(new Date(0)).getTime(), structuredClone([1,,3]));
        const cyc = {}; cyc.self = cyc; console.log(structuredClone(cyc).self === structuredClone(cyc));
        console.log('--- Array.fromAsync');
        console.log(typeof Array.fromAsync);
    "##;
    let expected = [
        "URLSearchParams { 'a' => '1', 'b' => '2' } [] {} 2",
        "3 [ [ 'a', '1' ], [ 'b', '2' ], [ 'c', '3' ] ]",
        "2 b=2&c=3",
        "[ [ 'x', '1' ] ] 2",
        "--- iterate other natives",
        "[ [ 'a', '1' ], [ 'b', '2' ] ]",
        "[ [ 'k', 'v' ] ]",
        "[ [ 1, 2 ] ] [ 1 ] [ 'a', 'b' ] [ 1, 2 ]",
        "[ 1n ]",
        "--- toLocaleString fallbacks",
        "1,234.568 0 -1,234",
        "1,a,true true",
        "--- freeze + accessors",
        "true false 1",
        "2",
        "--- structuredClone extras",
        "Map(1) { 1 => { a: 2 } } Set(2) { 1, 2 }",
        "0 [ 1, <1 empty item>, 3 ]",
        "false",
        "--- Array.fromAsync",
        "function",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}

#[test]
fn array_from_async_awaits_every_element() {
    // `Array.fromAsync` over an async iterable, a sync iterable whose elements
    // are promises, a bare async-generator iterator, an array-like, a string, a
    // Set, empty, and a rejection.
    let src = r##"
        (async()=>{
          console.log(await Array.fromAsync([1, Promise.resolve(2)]));
          console.log(await Array.fromAsync([1,2], async x => x*2));
          async function* g(){ yield 1; yield 2 }
          console.log(await Array.fromAsync(g()));
          console.log(await Array.fromAsync({length:2, 0:'a', 1:Promise.resolve('b')}));
          console.log(await Array.fromAsync('ab'));
          console.log(await Array.fromAsync(new Set([1,2])));
          console.log(await Array.fromAsync([]), typeof Array.fromAsync([]).then);
          try { await Array.fromAsync([Promise.reject(new Error('e'))]) } catch(e) { console.log('rejects', e.message) }
        })();
    "##;
    let expected = [
        "[ 1, 2 ]",
        "[ 2, 4 ]",
        "[ 1, 2 ]",
        "[ 'a', 'b' ]",
        "[ 'a', 'b' ]",
        "[ 1, 2 ]",
        "[] function",
        "rejects e",
    ]
    .join("\n");
    assert_eq!(run(src), expected);
}
