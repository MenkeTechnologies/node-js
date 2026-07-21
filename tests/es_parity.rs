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
