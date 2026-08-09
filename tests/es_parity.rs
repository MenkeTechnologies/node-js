//! Focused parity tests for the ECMAScript features fixed/added in the
//! change-by-copy + numeric/prototype sweep. Each expected value was captured
//! from system `node v26.5.0`; the tests drive the built `node` binary
//! (`CARGO_BIN_EXE_node`) as a subprocess so `console.log` output is exact and
//! no Node install is needed in CI. These pin behavior that the `examples/*.js`
//! snapshot does not already cover.

use std::io::Write;
use std::process::Command;

/// Run `src` through the built `node` binary, returning trimmed stdout. Panics
/// with stderr on a non-zero exit so a thrown error surfaces in the failure.
fn run(src: &str) -> String {
    let mut f = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .expect("temp file");
    f.write_all(src.as_bytes()).expect("write source");
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(f.path())
        .output()
        .expect("spawn node binary");
    if !out.status.success() {
        panic!(
            "program failed:\n--- stderr ---\n{}\n--- stdout ---\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
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
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(f.path())
        .output()
        .expect("spawn node binary");
    assert!(
        !out.status.success(),
        "expected a failure, got stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).trim_end().to_string()
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
    let out = Command::new(env!("CARGO_BIN_EXE_node"))
        .arg(f.path())
        .output()
        .expect("spawn node binary");
    assert!(
        !out.status.success(),
        "an unhandled rejection must not exit 0"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), "before");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("boom"),
        "stderr should name the rejection: {}",
        String::from_utf8_lossy(&out.stderr)
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
