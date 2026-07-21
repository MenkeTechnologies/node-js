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
