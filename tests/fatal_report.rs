//! What the built `node` writes to stderr when an exception ends the process,
//! and the `file:line:col` stack frames that report is made of.
//!
//! Every expectation is node v26.12.1's stderr for the same fixture with node's
//! own frames removed — the `node:internal/…` loader frames (`Module._compile`,
//! `wrapModuleLoad`, …), which this runtime does not have — and the version on
//! the last line taken from the runtime, which reports itself as
//! `process.version`. Nothing else is normalised: the source arrow, the caret
//! column, the blank lines, the inspected error and the exit status are exact.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A spawned `node` still running after this is stuck, not slow.
const CHILD_BUDGET: Duration = Duration::from_secs(60);

/// A fresh directory holding `files`, by its canonical path: node prints a
/// CommonJS entry by its realpath, which on macOS is `/private/var/…` for a
/// `/var/…` temp dir.
fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).expect("write fixture");
    }
    let canon = std::fs::canonicalize(dir.path()).expect("canonical temp dir");
    (dir, canon)
}

/// Run the built `node` with `args` in `cwd`; its exit code, stdout and
/// stderr.
fn run_node(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let out_path = cwd.join(".stdout");
    let err_path = cwd.join(".stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_node"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&out_path).expect("stdout file"))
        .stderr(std::fs::File::create(&err_path).expect("stderr file"))
        .spawn()
        .expect("spawn node binary");
    let deadline = Instant::now() + CHILD_BUDGET;
    let status = loop {
        if let Some(st) = child.try_wait().expect("wait for node binary") {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!(
                "node {args:?} did not exit within {}s",
                CHILD_BUDGET.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    (
        status.code().unwrap_or(-1),
        read(&out_path),
        read(&err_path),
    )
}

/// `expected` with `{dir}` replaced by the fixture directory and the
/// `Node.js {version}` trailer filled in.
fn expect(expected: &str, dir: &Path) -> String {
    expected
        .replace("{dir}", &dir.to_string_lossy())
        .replace("{version}", nodejs::stdlib::process::NODE_VERSION)
}

/// Run `entry` from a fixture of `files`; assert it exits 1 with exactly
/// `expected` on stderr.
fn assert_fatal(files: &[(&str, &str)], args: &[&str], expected: &str) {
    let (_keep, dir) = fixture(files);
    let (code, _, stderr) = run_node(&dir, args);
    assert_eq!(stderr, expect(expected, &dir), "stderr of node {args:?}");
    assert_eq!(code, 1, "exit status of node {args:?}");
}

#[test]
fn a_thrown_error_prints_the_source_arrow_then_its_stack() {
    assert_fatal(
        &[(
            "nested.js",
            "function inner() {\n  throw new TypeError(\"boom\");\n}\nfunction outer() { inner(); }\nouter();\n",
        )],
        &["nested.js"],
        "{dir}/nested.js:2\n  throw new TypeError(\"boom\");\n  ^\n\n\
         TypeError: boom\n    at inner ({dir}/nested.js:2:9)\n    at outer ({dir}/nested.js:4:20)\n    at Object.<anonymous> ({dir}/nested.js:5:1)\n\n\
         Node.js {version}\n",
    );
}

#[test]
fn a_runtime_type_error_points_at_the_property() {
    assert_fatal(
        &[("nullread.js", "const cfg = null;\nconsole.log(cfg.port);\n")],
        &["nullread.js"],
        "{dir}/nullread.js:2\nconsole.log(cfg.port);\n                ^\n\n\
         TypeError: Cannot read properties of null (reading 'port')\n    at Object.<anonymous> ({dir}/nullread.js:2:17)\n\n\
         Node.js {version}\n",
    );
}

#[test]
fn an_errors_own_properties_follow_its_stack() {
    assert_fatal(
        &[(
            "props.js",
            "const e = new Error(\"bad\");\ne.code = \"E_BAD\";\nthrow e;\n",
        )],
        &["props.js"],
        "{dir}/props.js:3\nthrow e;\n^\n\n\
         Error: bad\n    at Object.<anonymous> ({dir}/props.js:1:11) {\n  code: 'E_BAD'\n}\n\n\
         Node.js {version}\n",
    );
}

#[test]
fn a_thrown_primitive_gets_the_trace_uncaught_hint() {
    assert_fatal(
        &[],
        &["-e", "throw \"s\""],
        "\n[eval]:1\nthrow \"s\"\n^\ns\n\
         (Use `node --trace-uncaught ...` to show where the exception was thrown)\n\n\
         Node.js {version}\n",
    );
}

/// A non-error object thrown from a file is printed by `AppendExceptionLine`
/// behind a blank line, with no blank line before the value; the caret line
/// copies the source line's tab.
#[test]
fn a_thrown_plain_object_is_printed_after_the_arrow() {
    assert_fatal(
        &[("object.js", "const point = { x: 1 };\n\tthrow point;\n")],
        &["object.js"],
        "\n{dir}/object.js:2\n\tthrow point;\n\t^\n{ x: 1 }\n\nNode.js {version}\n",
    );
}

#[test]
fn an_unhandled_rejection_points_where_the_error_was_created() {
    assert_fatal(
        &[(
            "reject.js",
            "function load() {\n  return Promise.reject(new RangeError(\"late\"));\n}\nload();\n",
        )],
        &["reject.js"],
        "{dir}/reject.js:2\n  return Promise.reject(new RangeError(\"late\"));\n                        ^\n\n\
         RangeError: late\n    at load ({dir}/reject.js:2:25)\n    at Object.<anonymous> ({dir}/reject.js:4:1)\n\n\
         Node.js {version}\n",
    );
}

/// A required module's frames name its own file, counting lines from its
/// first line rather than the wrapper's, and its body runs on
/// `module.exports` — so it too is `Object.<anonymous>`.
#[test]
fn a_required_module_reports_its_own_file() {
    assert_fatal(
        &[
            ("main.js", "require(\"./lib.js\");\n"),
            ("lib.js", "function parse() {\n  throw new SyntaxError(\"nope\");\n}\nparse();\n"),
        ],
        &["main.js"],
        "{dir}/lib.js:2\n  throw new SyntaxError(\"nope\");\n  ^\n\n\
         SyntaxError: nope\n    at parse ({dir}/lib.js:2:9)\n    at Object.<anonymous> ({dir}/lib.js:4:1)\n    at Object.<anonymous> ({dir}/main.js:1:1)\n\n\
         Node.js {version}\n",
    );
}

#[test]
fn a_syntax_error_points_at_the_offending_token() {
    assert_fatal(
        &[("syntax.js", "const a = 1;\nconst b = a +;\n")],
        &["syntax.js"],
        "{dir}/syntax.js:2\nconst b = a +;\n             ^\n\nSyntaxError: Unexpected token ';'\n\nNode.js {version}\n",
    );
}

#[test]
fn an_eval_script_has_unnamed_top_level_frames() {
    assert_fatal(
        &[],
        &["-e", "null.x"],
        "[eval]:1\nnull.x\n     ^\n\n\
         TypeError: Cannot read properties of null (reading 'x')\n    at [eval]:1:6\n\n\
         Node.js {version}\n",
    );
}

/// An ES module's evaluation error reaches node's report through a promise, so
/// the caret is where the error was created (`new`), not the `throw`.
#[test]
fn an_es_module_reports_its_file_url() {
    assert_fatal(
        &[("mod.mjs", "const n = 1;\nthrow new Error(\"esm \" + n);\n")],
        &["mod.mjs"],
        "file://{dir}/mod.mjs:2\nthrow new Error(\"esm \" + n);\n      ^\n\n\
         Error: esm 1\n    at file://{dir}/mod.mjs:2:7\n\n\
         Node.js {version}\n",
    );
}

/// node runs the `exit` listeners inside `process._fatalException`, before the
/// report is printed.
#[test]
fn exit_listeners_run_before_the_report() {
    let (_keep, dir) = fixture(&[(
        "exit.js",
        "process.on(\"exit\", (c) => console.error(\"exit\", c));\nthrow new Error(\"z\");\n",
    )]);
    let (code, _, stderr) = run_node(&dir, &["exit.js"]);
    assert_eq!(
        stderr,
        expect(
            "exit 1\n{dir}/exit.js:2\nthrow new Error(\"z\");\n^\n\n\
             Error: z\n    at Object.<anonymous> ({dir}/exit.js:2:7)\n\nNode.js {version}\n",
            &dir
        )
    );
    assert_eq!(code, 1);
}

/// V8's frame names: `new C` for a construct call, `Type.m` for a method (a
/// static one is typed by its class), a bare `get x` for an accessor, and an
/// async function's synchronous part still showing its caller.
#[test]
fn stack_frames_are_named_as_v8_names_them() {
    let (_keep, dir) = fixture(&[(
        "frames.js",
        "class Shape {\n  static make() { return new Shape(); }\n  constructor() { this.e = new Error(\"s\"); }\n  \
         get area() { return new Error(\"a\"); }\n  draw() { return new Error(\"d\"); }\n}\n\
         const obj = { run() { return new Error(\"r\"); } };\nasync function job() { return new Error(\"j\"); }\n\
         for (const e of [Shape.make().e, new Shape().area, new Shape().draw(), obj.run()]) console.log(e.stack.split(\"\\n\").slice(0, 3).join(\"|\"));\n\
         job().then(e => console.log(e.stack.split(\"\\n\").slice(0, 3).join(\"|\")));\n",
    )]);
    let (code, stdout, stderr) = run_node(&dir, &["frames.js"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        expect(
            "Error: s|    at new Shape ({dir}/frames.js:3:28)|    at Shape.make ({dir}/frames.js:2:26)\n\
             Error: a|    at get area ({dir}/frames.js:4:23)|    at Object.<anonymous> ({dir}/frames.js:9:46)\n\
             Error: d|    at Shape.draw ({dir}/frames.js:5:19)|    at Object.<anonymous> ({dir}/frames.js:9:64)\n\
             Error: r|    at Object.run ({dir}/frames.js:7:30)|    at Object.<anonymous> ({dir}/frames.js:9:76)\n\
             Error: j|    at job ({dir}/frames.js:8:31)|    at Object.<anonymous> ({dir}/frames.js:10:1)\n",
            &dir
        )
    );
}

/// node calls a module's wrapper on `module.exports`, which is also what makes
/// its frames `Object.<anonymous>`.
#[test]
fn a_required_modules_this_is_its_exports() {
    let (_keep, dir) = fixture(&[
        ("entry.js", "require(\"./dep.js\");\n"),
        (
            "dep.js",
            "console.log(this === module.exports, this === globalThis);\n",
        ),
    ]);
    let (code, stdout, stderr) = run_node(&dir, &["entry.js"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "true false\n");
}
