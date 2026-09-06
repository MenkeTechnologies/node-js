//! The structural diff Node prints inside an `AssertionError` message.
//!
//! A failing `deepStrictEqual`/`strictEqual` in Node does not read `{ a: 1 } !==
//! { a: 2 }`; it renders both operands with `util.inspect` and prints a
//! line-oriented diff of the two renderings, marking lines only in `actual` with
//! `+` and lines only in `expected` with `-`:
//!
//! ```text
//! Expected values to be strictly deep-equal:
//! + actual - expected
//!
//!   {
//!     a: 1,
//! +   b: 2
//! -   b: 3
//!   }
//! ```
//!
//! This is a port of `lib/internal/assert/myers_diff.js` and the `createErrDiff`
//! half of `lib/internal/assert/assertion_error.js`. The diff itself is Myers'
//! O(ND) algorithm; the printer is the part with all the observable detail
//! (which lines are context, when a long identical run collapses to `...`, and
//! the `... Skipped lines` note that then appears in the header).
//!
//! Colors are never emitted: the harness compares captured stdout, which Node
//! itself renders uncolored when stderr is not a TTY.

use crate::host::with_host;
use fusevm::Value;

/// One line of the merged diff. `Nop` lines are present in both renderings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Delete,
    Nop,
    Insert,
}

/// How many consecutive unchanged lines are printed before the run collapses.
/// Node's `kNopLinesToCollapse`.
const NOP_LINES_TO_COLLAPSE: usize = 5;

/// Node's `kMaxShortStringLength`: below this combined width the two operands
/// are printed as `actual !== expected` on one line instead of diffed.
const MAX_SHORT_STRING_LENGTH: usize = 12;

/// Render a value the way `assert` does, which is NOT the way `console.log`
/// does: every group broken onto its own line (`compact: false`), keys in sorted
/// order, no depth or array-length limit. The options matter to the diff rather
/// than to taste — one property per line is what gives the differ something to
/// match line-by-line, and sorting means two objects built with the same
/// properties in a different insertion order diff as equal instead of as a
/// wholesale rewrite.
///
/// `customInspect: false` matters more than it sounds: it is what makes a
/// Buffer in a failing assertion print its bytes as a list rather than as the
/// opaque `<Buffer ff fe …>` summary, so the diff can point at the byte that
/// differs. Node's `showProxy: false` is already this renderer's behavior.
fn inspect_value(v: &Value) -> String {
    crate::host::set_inspect_compact(0);
    crate::host::set_inspect_sorted(true);
    crate::host::set_inspect_max_depth(1000);
    crate::host::set_inspect_max_array_length(usize::MAX);
    crate::host::set_inspect_custom(false);
    let s = with_host(|h| h.inspect(v));
    crate::host::set_inspect_compact(3);
    crate::host::set_inspect_sorted(false);
    crate::host::set_inspect_max_depth(2);
    crate::host::set_inspect_max_array_length(crate::host::DEFAULT_MAX_ARRAY_LENGTH);
    crate::host::set_inspect_custom(true);
    s
}

/// Whether two rendered lines count as the same line.
///
/// `check_comma_disparity` makes a line equal to itself-plus-a-trailing-comma.
/// Without it, appending a property to an object would report the previous last
/// property as changed too, purely because it gained a separator: diffing
/// `{a: 1}` against `{a: 1, b: 2}` would mark `a: 1` -> `a: 1,` as a
/// substitution rather than leaving it as context.
fn lines_equal(actual: &str, expected: &str, check_comma_disparity: bool) -> bool {
    if actual == expected {
        return true;
    }
    if check_comma_disparity {
        return expected.strip_suffix(',').is_some_and(|e| e == actual)
            || actual.strip_suffix(',').is_some_and(|a| a == expected);
    }
    false
}

/// Myers' shortest-edit-script over the two line lists, returned in REVERSE
/// order (the printer walks it back to front, as Node's does).
///
/// The `v` array is indexed by diagonal. Node reads `v[offset - 1]` and
/// `v[offset + 1]` out of an `Int32Array`, where an out-of-range read yields
/// `undefined` and every comparison against it is false; each such read here is
/// instead guarded by the branch that already made it unreachable, so no
/// sentinel value is needed.
fn myers_diff(
    actual: &[&str],
    expected: &[&str],
    check_comma_disparity: bool,
) -> Vec<(Op, String)> {
    let actual_len = actual.len() as i64;
    let expected_len = expected.len() as i64;
    let max = actual_len + expected_len;
    // Both sides empty: no edit script, and the loop below would index `v[max]`
    // of a one-element array looking for a match it already has.
    if max == 0 {
        return Vec::new();
    }
    let mut v = vec![0i64; (2 * max + 1) as usize];
    let mut trace: Vec<Vec<i64>> = Vec::new();

    for diff_level in 0..=max {
        trace.push(v.clone());
        let mut diagonal = -diff_level;
        while diagonal <= diff_level {
            let offset = (diagonal + max) as usize;
            // `diagonal == -diff_level` short-circuits before `v[offset - 1]` is
            // read, which is the only case in which `offset` can be 0.
            let mut x = if diagonal == -diff_level
                || (diagonal != diff_level && v[offset - 1] < v[offset + 1])
            {
                v[offset + 1]
            } else {
                v[offset - 1] + 1
            };
            let mut y = x - diagonal;

            while x < actual_len
                && y < expected_len
                && lines_equal(
                    actual[x as usize],
                    expected[y as usize],
                    check_comma_disparity,
                )
            {
                x += 1;
                y += 1;
            }

            v[offset] = x;

            if x >= actual_len && y >= expected_len {
                return backtrack(&trace, actual, expected, check_comma_disparity);
            }
            diagonal += 2;
        }
    }
    Vec::new()
}

/// Walk the recorded per-level diagonals back to the origin, emitting the edit
/// script. Returned reversed, exactly as Node returns it.
fn backtrack(
    trace: &[Vec<i64>],
    actual: &[&str],
    expected: &[&str],
    check_comma_disparity: bool,
) -> Vec<(Op, String)> {
    let actual_len = actual.len() as i64;
    let expected_len = expected.len() as i64;
    let max = actual_len + expected_len;

    let mut x = actual_len;
    let mut y = expected_len;
    let mut result: Vec<(Op, String)> = Vec::new();

    for diff_level in (0..trace.len() as i64).rev() {
        let v = &trace[diff_level as usize];
        let diagonal = x - y;
        let offset = (diagonal + max) as usize;

        let prev_diagonal = if diagonal == -diff_level
            || (diagonal != diff_level && v[offset - 1] < v[offset + 1])
        {
            diagonal + 1
        } else {
            diagonal - 1
        };

        // Node reads this out of bounds on the last level and gets `undefined`,
        // which makes both `x > prevX` tests below false. `None` reproduces that
        // without inventing a numeric sentinel that could compare as a real
        // position.
        let idx = prev_diagonal + max;
        let prev = if idx >= 0 && (idx as usize) < v.len() {
            let prev_x = v[idx as usize];
            Some((prev_x, prev_x - prev_diagonal))
        } else {
            None
        };

        if let Some((prev_x, prev_y)) = prev {
            while x > prev_x && y > prev_y {
                let actual_item = actual[(x - 1) as usize];
                // With comma disparity on, a context line is printed in the
                // EXPECTED side's spelling when the actual side is the one
                // missing the trailing comma — otherwise the last property of a
                // shortened object would print without the separator the
                // surrounding lines still carry.
                let value = if check_comma_disparity && !actual_item.ends_with(',') {
                    expected[(y - 1) as usize]
                } else {
                    actual_item
                };
                result.push((Op::Nop, value.to_string()));
                x -= 1;
                y -= 1;
            }
        }

        if diff_level > 0 {
            if prev.is_some_and(|(prev_x, _)| x > prev_x) {
                x -= 1;
                result.push((Op::Insert, actual[x as usize].to_string()));
            } else {
                y -= 1;
                result.push((Op::Delete, expected[y as usize].to_string()));
            }
        }
    }
    result
}

/// Render the edit script. Returns the message body and whether any unchanged
/// run was collapsed (which the caller reports as `... Skipped lines`).
///
/// Unchanged lines are printed only while fewer than `NOP_LINES_TO_COLLAPSE` of
/// them have accumulated since the last change. When the run then ends, its LAST
/// one or two lines are printed after the fact so that a change keeps its
/// leading context, and a longer run is introduced by a bare `...`.
fn print_myers_diff(diff: &[(Op, String)]) -> (String, bool) {
    let mut message = String::new();
    let mut skipped = false;
    let mut nop_count = 0usize;

    for idx in (0..diff.len()).rev() {
        let (operation, value) = (&diff[idx].0, &diff[idx].1);
        let previous_operation = if idx + 1 < diff.len() {
            Some(diff[idx + 1].0)
        } else {
            None
        };

        // A run of unchanged lines has just ended. Re-emit its tail as leading
        // context for the change about to be printed — but only if that is more
        // than one line, since collapsing a single line saves nothing.
        if previous_operation == Some(Op::Nop) && *operation != Op::Nop {
            if nop_count == NOP_LINES_TO_COLLAPSE + 1 {
                message.push_str(&format!("  {}\n", diff[idx + 1].1));
            } else if nop_count == NOP_LINES_TO_COLLAPSE + 2 {
                message.push_str(&format!("  {}\n", diff[idx + 2].1));
                message.push_str(&format!("  {}\n", diff[idx + 1].1));
            } else if nop_count >= NOP_LINES_TO_COLLAPSE + 3 {
                message.push_str("...\n");
                message.push_str(&format!("  {}\n", diff[idx + 1].1));
                skipped = true;
            }
            nop_count = 0;
        }

        match operation {
            Op::Insert => message.push_str(&format!("+ {value}\n")),
            Op::Delete => message.push_str(&format!("- {value}\n")),
            Op::Nop => {
                if nop_count < NOP_LINES_TO_COLLAPSE {
                    message.push_str(&format!("  {value}\n"));
                }
                nop_count += 1;
            }
        }
    }

    (message.trim_end().to_string(), skipped)
}

/// Node's `kReadableOperator`: the heading each comparison writes above its diff.
fn readable_operator(op: &str) -> &'static str {
    match op {
        "deepStrictEqual" => "Expected values to be strictly deep-equal:",
        "partialDeepStrictEqual" => "Expected values to be partially and strictly deep-equal:",
        "strictEqual" => "Expected values to be strictly equal:",
        "strictEqualObject" => "Expected \"actual\" to be reference-equal to \"expected\":",
        "notIdentical" => "Values have same structure but are not reference-equal:",
        _ => "Expected values to be strictly deep-equal:",
    }
}

/// Whether a value is an object for the purposes below — `typeof v === 'object'
/// && v !== null`, so a function is NOT one.
fn is_object(v: &Value) -> bool {
    with_host(|h| h.type_of(v) == "object" && !h.is_null(v))
}

/// Node's `checkOperator`: `strictEqual` between two distinct objects reports
/// that they are not REFERENCE-equal, since their contents may well match.
fn check_operator(actual: &Value, expected: &Value, operator: &str) -> String {
    if operator == "strictEqual" && is_object(actual) && is_object(expected) {
        return "strictEqualObject".to_string();
    }
    operator.to_string()
}

/// Node's `getStackedDiff`: the two operands stacked on their own `+`/`-` lines,
/// with a caret under the first differing character when both sides are short
/// enough to line up.
fn stacked_diff(actual: &str, expected: &str) -> String {
    let mut message = format!("\n+ {actual}\n- {expected}");
    // Node compares against the terminal width only when stderr is a TTY, and
    // otherwise against 80. Assert messages are captured, not printed, in every
    // context this runtime is compared in, so 80 is the constant here.
    let strings_len = actual.chars().count() + expected.chars().count();
    if strings_len > 80 {
        return message;
    }
    let a: Vec<char> = actual.chars().collect();
    let e: Vec<char> = expected.chars().collect();
    let mut indicator_idx: Option<usize> = None;
    for i in 0..a.len() {
        if e.get(i) != Some(&a[i]) {
            // The first two characters are skipped because a difference there is
            // already obvious; the bound is 3 rather than 2 to account for the
            // opening quote a rendered string carries.
            if i >= 3 {
                indicator_idx = Some(i);
            }
            break;
        }
    }
    if let Some(i) = indicator_idx {
        message.push('\n');
        message.push_str(&" ".repeat(i + 2));
        message.push('^');
    }
    message
}

/// Node's `getSimpleDiff`: how two operands that each rendered to a SINGLE line
/// are shown.
fn simple_diff(
    original_actual: &Value,
    actual: &str,
    original_expected: &Value,
    expected: &str,
) -> (String, Option<String>) {
    let is_str = |v: &Value| with_host(|h| h.type_of(v) == "string");
    let mut strings_len = actual.chars().count() + expected.chars().count();
    // A rendered string carries quotes the comparison should not count.
    if is_str(original_actual) {
        strings_len = strings_len.saturating_sub(2);
    }
    if is_str(original_expected) {
        strings_len = strings_len.saturating_sub(2);
    }
    let both_zero = with_host(|h| {
        h.type_of(original_actual) == "number"
            && h.type_of(original_expected) == "number"
            && h.to_number(original_actual) == 0.0
            && h.to_number(original_expected) == 0.0
    });
    // `0` against `-0` renders as `0` and `-0`, which is short enough for the
    // one-line form — but that form would read `0 !== -0` with no indication of
    // which is which, so Node stacks it instead.
    if strings_len <= MAX_SHORT_STRING_LENGTH && !both_zero {
        return (format!("{actual} !== {expected}"), Some(String::new()));
    }
    (stacked_diff(actual, expected), None)
}

/// Build the whole `AssertionError` message for the comparisons that carry a
/// diff (`strictEqual`, `deepStrictEqual`, `partialDeepStrictEqual`).
///
/// A custom message replaces only the HEADING; Node still appends the diff, so
/// `assert.deepStrictEqual(a, b, 'boom')` reports `boom` followed by the same
/// `+ actual - expected` body a generated message would carry.
pub fn create_err_diff(
    actual: &Value,
    expected: &Value,
    operator: &str,
    custom_message: Option<&str>,
) -> String {
    let mut operator = check_operator(actual, expected, operator);
    let mut skipped = false;
    let message;
    let inspected_actual = inspect_value(actual);
    let inspected_expected = inspect_value(expected);
    let split_actual: Vec<&str> = inspected_actual.split('\n').collect();
    let split_expected: Vec<&str> = inspected_expected.split('\n').collect();
    let mut header = "+ actual - expected".to_string();

    // Node's `isSimpleDiff`: a diff needs something to align, so it applies only
    // once at least one side rendered to more than one line AND both sides are
    // objects.
    let show_simple = (split_actual.len() > 1 || split_expected.len() > 1)
        .then_some(false)
        .unwrap_or(!is_object(actual) || !is_object(expected));

    if show_simple {
        let (m, h) = simple_diff(actual, split_actual[0], expected, split_expected[0]);
        message = m;
        if let Some(h) = h {
            header = h;
        }
    } else if inspected_actual == inspected_expected {
        // Structurally identical but not the same reference — there is no diff
        // to draw, so Node prints the one shared rendering under a heading that
        // says exactly that.
        operator = "notIdentical".to_string();
        if split_actual.len() > 50 {
            message = format!("{}\n...}}", split_actual[..50].join("\n"));
            skipped = true;
        } else {
            message = split_actual.join("\n");
        }
        header = String::new();
    } else {
        // Comma disparity is only meaningful between two rendered containers;
        // between primitives a trailing comma is part of the value.
        let check_comma_disparity = is_object(actual);
        let diff = myers_diff(&split_actual, &split_expected, check_comma_disparity);
        let (body, was_skipped) = print_myers_diff(&diff);
        message = format!("\n{body}");
        if was_skipped {
            skipped = true;
        }
    }

    let heading = custom_message
        .map(str::to_string)
        .unwrap_or_else(|| readable_operator(&operator).to_string());
    let skipped_message = if skipped { "\n... Skipped lines" } else { "" };
    format!("{heading}\n{header}{skipped_message}\n{message}\n")
}

/// Render a value for the assert messages that show ONE operand rather than a
/// diff (`notDeepStrictEqual`, `deepEqual`, `notDeepEqual`). These use the same
/// expanded, sorted rendering as the diff body, not `console.log`'s.
pub fn inspect_operand(v: &Value) -> String {
    inspect_value(v)
}
