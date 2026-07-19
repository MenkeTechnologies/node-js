//! Builtin / keyword / method corpus for offline documentation.
//!
//! Single source of truth for the reference manual (`gen-docs` → `docs/reference.html`).
//! Every entry mirrors something the runtime actually recognizes:
//!   * "Keyword"      → the `KEYWORDS` set in `parser.rs` (plus the operator
//!     keywords `typeof`/`void`/`delete`/`instanceof`/`in`/`new`/`this`).
//!   * "Global"       → the global identifiers resolved in `builtins.rs`
//!     (`undefined`/`NaN`/`Infinity`/`globalThis`) and the free functions /
//!     constructors in its dispatch table (`parseInt`, `parseFloat`, `isNaN`,
//!     `isFinite`, `String`, `Number`, `Boolean`, `Array`, `Error`, …).
//!   * "console"      → `console.log`/`error`/`warn`/`info`/`debug`.
//!   * "Math"/"JSON"/"Object"/"Number"/"String static"/"Array static" → the
//!     namespace dispatch arms in `builtins.rs` (`call_builtin`, `call_math`,
//!     the `Math.*` const table, `Object.*`, `Number.*`, `Array.*`).
//!   * "Array method"/"String method"/"Number method" → the per-type dispatch
//!     tables (`array_method`, `string_method`, and the number-method arms).
//!
//! Only names the crate implements appear here — no classes, regex, Map/Set,
//! generators, or modules, none of which the lexer/parser/builtins support.

/// The builtin corpus: (name, chapter, one-line doc, runnable example).
const CORPUS: &[(&str, &str, &str, &str)] = &[
    // ── Keyword ──
    (
        "var",
        "Keyword",
        "declare a function-scoped (hoisted) variable",
        "var x = 1; x   // => 1",
    ),
    (
        "let",
        "Keyword",
        "declare a block-scoped variable",
        "let x = 2; x   // => 2",
    ),
    (
        "const",
        "Keyword",
        "declare a block-scoped binding that cannot be reassigned",
        "const x = 3; x   // => 3",
    ),
    (
        "function",
        "Keyword",
        "define a function (declaration or expression)",
        "function f() { return 9; } f()   // => 9",
    ),
    (
        "return",
        "Keyword",
        "return a value from the current function (undefined if omitted)",
        "(function () { return 7; })()   // => 7",
    ),
    (
        "if",
        "Keyword",
        "conditional branch on a truthy test",
        "let x; if (true) x = 1; x   // => 1",
    ),
    (
        "else",
        "Keyword",
        "fallback branch of an if",
        "let x; if (false) x = 1; else x = 2; x   // => 2",
    ),
    (
        "while",
        "Keyword",
        "loop while the condition is truthy",
        "let i = 0; while (i < 3) i++; i   // => 3",
    ),
    (
        "do",
        "Keyword",
        "do/while loop: run the body once, then repeat while truthy",
        "let i = 0; do { i++; } while (i < 3); i   // => 3",
    ),
    (
        "for",
        "Keyword",
        "C-style loop, or for/of and for/in iteration",
        "let s = 0; for (let i = 0; i < 3; i++) s += i; s   // => 3",
    ),
    (
        "of",
        "Keyword",
        "iterate values of an iterable: `for (x of iterable)`",
        "let s = 0; for (const n of [1, 2, 3]) s += n; s   // => 6",
    ),
    (
        "in",
        "Keyword",
        "for/in key iteration, and the property-membership operator",
        "\"a\" in { a: 1 }   // => true",
    ),
    (
        "switch",
        "Keyword",
        "multi-way branch on a discriminant with case labels",
        "let x; switch (2) { case 2: x = \"b\"; break; } x   // => 'b'",
    ),
    (
        "case",
        "Keyword",
        "a labeled branch inside a switch",
        "let x; switch (1) { case 1: x = \"a\"; } x   // => 'a'",
    ),
    (
        "default",
        "Keyword",
        "the fallback branch of a switch",
        "let x; switch (9) { default: x = \"d\"; } x   // => 'd'",
    ),
    (
        "break",
        "Keyword",
        "exit the nearest enclosing loop or switch",
        "let x; for (const n of [1, 2, 3]) { if (n === 2) break; x = n; } x   // => 1",
    ),
    (
        "continue",
        "Keyword",
        "skip to the next iteration of the nearest loop",
        "let s = 0; for (const n of [1, 2, 3]) { if (n === 2) continue; s += n; } s   // => 4",
    ),
    (
        "new",
        "Keyword",
        "construct an instance: `new Ctor(args)`",
        "new Error(\"boom\").message   // => 'boom'",
    ),
    (
        "this",
        "Keyword",
        "the receiver of the current call",
        "({ n: 5, get() { return this.n; } }).get()   // => 5",
    ),
    (
        "typeof",
        "Keyword",
        "the type tag of a value as a string",
        "typeof 3   // => 'number'",
    ),
    (
        "void",
        "Keyword",
        "evaluate an expression and yield undefined",
        "void 0   // => undefined",
    ),
    (
        "delete",
        "Keyword",
        "remove a property from an object",
        "const o = { a: 1 }; delete o.a; o.a   // => undefined",
    ),
    (
        "instanceof",
        "Keyword",
        "test whether an object was built by a constructor",
        "new Error(\"e\") instanceof Error   // => true",
    ),
    (
        "throw",
        "Keyword",
        "raise an exception value",
        "try { throw \"x\"; } catch (e) { e }   // => 'x'",
    ),
    (
        "try",
        "Keyword",
        "run a block, routing exceptions to catch/finally",
        "let x; try { x = 1; } finally { x = 2; } x   // => 2",
    ),
    (
        "catch",
        "Keyword",
        "handle an exception thrown in the try block",
        "try { null.x; } catch (e) { \"caught\" }   // => 'caught'",
    ),
    (
        "finally",
        "Keyword",
        "block that always runs after try/catch",
        "let x; try { x = 1; } finally { x = 3; } x   // => 3",
    ),
    (
        "true",
        "Keyword",
        "the boolean true literal",
        "true && 1   // => 1",
    ),
    (
        "false",
        "Keyword",
        "the boolean false literal",
        "false || 2   // => 2",
    ),
    (
        "null",
        "Keyword",
        "the intentional-absence value",
        "null ?? 5   // => 5",
    ),
    // ── Global ──
    (
        "undefined",
        "Global",
        "the value of an unassigned binding or missing property",
        "typeof undefined   // => 'undefined'",
    ),
    (
        "NaN",
        "Global",
        "the not-a-number float; unequal to itself",
        "NaN === NaN   // => false",
    ),
    (
        "Infinity",
        "Global",
        "the positive-infinity float",
        "1 / 0 === Infinity   // => true",
    ),
    (
        "globalThis",
        "Global",
        "the global object",
        "typeof globalThis   // => 'object'",
    ),
    (
        "parseInt",
        "Global",
        "parse the leading integer of a string (optional radix)",
        "parseInt(\"42px\")   // => 42",
    ),
    (
        "parseFloat",
        "Global",
        "parse the leading floating-point number of a string",
        "parseFloat(\"3.14abc\")   // => 3.14",
    ),
    (
        "isNaN",
        "Global",
        "true if the coerced number is NaN",
        "isNaN(\"x\")   // => true",
    ),
    (
        "isFinite",
        "Global",
        "true if the coerced number is finite",
        "isFinite(10)   // => true",
    ),
    (
        "String",
        "Global",
        "convert a value to its string form",
        "String(123)   // => '123'",
    ),
    (
        "Number",
        "Global",
        "convert a value to a number",
        "Number(\"3.5\")   // => 3.5",
    ),
    (
        "Boolean",
        "Global",
        "convert a value to its truthiness",
        "Boolean(\"\")   // => false",
    ),
    (
        "Array",
        "Global",
        "build an array from the given elements",
        "Array(1, 2, 3)   // => [ 1, 2, 3 ]",
    ),
    (
        "Error",
        "Global",
        "construct a generic error with a message",
        "new Error(\"boom\").message   // => 'boom'",
    ),
    (
        "TypeError",
        "Global",
        "construct a type error (name is 'TypeError')",
        "new TypeError(\"bad\").name   // => 'TypeError'",
    ),
    (
        "RangeError",
        "Global",
        "construct a range error (name is 'RangeError')",
        "new RangeError(\"oob\").name   // => 'RangeError'",
    ),
    // ── console ──
    (
        "console.log",
        "console",
        "write args to stdout, space-separated, ending in a newline",
        "console.log(\"hi\", 1)   // prints: hi 1",
    ),
    (
        "console.error",
        "console",
        "write args to stderr",
        "console.error(\"oops\")   // prints oops to stderr",
    ),
    (
        "console.warn",
        "console",
        "write args to stderr (warning channel)",
        "console.warn(\"careful\")   // prints careful to stderr",
    ),
    (
        "console.info",
        "console",
        "write args to stdout (info channel)",
        "console.info(\"note\")   // prints note",
    ),
    (
        "console.debug",
        "console",
        "write args to stdout (debug channel)",
        "console.debug(\"dbg\")   // prints dbg",
    ),
    // ── Math ──
    (
        "Math.PI",
        "Math",
        "the ratio of a circle's circumference to its diameter",
        "Math.PI   // => 3.141592653589793",
    ),
    (
        "Math.abs",
        "Math",
        "absolute value of a number",
        "Math.abs(-5)   // => 5",
    ),
    (
        "Math.floor",
        "Math",
        "largest integer <= x",
        "Math.floor(3.7)   // => 3",
    ),
    (
        "Math.ceil",
        "Math",
        "smallest integer >= x",
        "Math.ceil(3.2)   // => 4",
    ),
    (
        "Math.round",
        "Math",
        "round to the nearest integer (ties toward +Infinity)",
        "Math.round(2.5)   // => 3",
    ),
    (
        "Math.trunc",
        "Math",
        "the integer part of x, dropping any fraction",
        "Math.trunc(-4.7)   // => -4",
    ),
    (
        "Math.sign",
        "Math",
        "the sign of x as -1, 0, or 1",
        "Math.sign(-8)   // => -1",
    ),
    (
        "Math.sqrt",
        "Math",
        "square root of x",
        "Math.sqrt(16)   // => 4",
    ),
    (
        "Math.cbrt",
        "Math",
        "cube root of x",
        "Math.cbrt(27)   // => 3",
    ),
    (
        "Math.pow",
        "Math",
        "x raised to the power y",
        "Math.pow(2, 10)   // => 1024",
    ),
    (
        "Math.exp",
        "Math",
        "e raised to the power x",
        "Math.exp(0)   // => 1",
    ),
    (
        "Math.log",
        "Math",
        "natural logarithm of x",
        "Math.log(1)   // => 0",
    ),
    (
        "Math.max",
        "Math",
        "largest of the arguments",
        "Math.max(3, 1, 2)   // => 3",
    ),
    (
        "Math.min",
        "Math",
        "smallest of the arguments",
        "Math.min(3, 1, 2)   // => 1",
    ),
    (
        "Math.hypot",
        "Math",
        "the square root of the sum of squares of the arguments",
        "Math.hypot(3, 4)   // => 5",
    ),
    (
        "Math.sin",
        "Math",
        "sine of x (radians)",
        "Math.sin(0)   // => 0",
    ),
    (
        "Math.cos",
        "Math",
        "cosine of x (radians)",
        "Math.cos(0)   // => 1",
    ),
    (
        "Math.tan",
        "Math",
        "tangent of x (radians)",
        "Math.tan(0)   // => 0",
    ),
    (
        "Math.random",
        "Math",
        "a pseudo-random float in [0, 1)",
        "Math.random() < 1   // => true",
    ),
    // ── JSON ──
    (
        "JSON.stringify",
        "JSON",
        "serialize a value to a JSON string",
        "JSON.stringify({ a: 1 })   // => '{\"a\":1}'",
    ),
    (
        "JSON.parse",
        "JSON",
        "parse a JSON string into a value",
        "JSON.parse(\"[1,2]\")   // => [ 1, 2 ]",
    ),
    // ── Object ──
    (
        "Object.keys",
        "Object",
        "an array of an object's own enumerable keys",
        "Object.keys({ a: 1, b: 2 })   // => [ 'a', 'b' ]",
    ),
    (
        "Object.values",
        "Object",
        "an array of an object's own enumerable values",
        "Object.values({ a: 1, b: 2 })   // => [ 1, 2 ]",
    ),
    (
        "Object.entries",
        "Object",
        "an array of [key, value] pairs",
        "Object.entries({ a: 1 })   // => [ [ 'a', 1 ] ]",
    ),
    (
        "Object.assign",
        "Object",
        "copy source properties onto a target object (in place)",
        "Object.assign({ a: 1 }, { b: 2 })   // => { a: 1, b: 2 }",
    ),
    (
        "Object.fromEntries",
        "Object",
        "build an object from [key, value] pairs",
        "Object.fromEntries([[\"a\", 1]])   // => { a: 1 }",
    ),
    (
        "Object.freeze",
        "Object",
        "make an object immutable and return it",
        "Object.freeze({ a: 1 }).a   // => 1",
    ),
    // ── Number ──
    (
        "Number.isInteger",
        "Number",
        "true if the value is an integer number",
        "Number.isInteger(3)   // => true",
    ),
    (
        "Number.isNaN",
        "Number",
        "true only if the value is exactly NaN (no coercion)",
        "Number.isNaN(NaN)   // => true",
    ),
    (
        "Number.isFinite",
        "Number",
        "true if the value is a finite number (no coercion)",
        "Number.isFinite(10)   // => true",
    ),
    (
        "Number.isSafeInteger",
        "Number",
        "true if the value is an integer within +/-2^53-1",
        "Number.isSafeInteger(2 ** 53)   // => false",
    ),
    (
        "Number.parseInt",
        "Number",
        "same as the global parseInt",
        "Number.parseInt(\"20\", 10)   // => 20",
    ),
    (
        "Number.parseFloat",
        "Number",
        "same as the global parseFloat",
        "Number.parseFloat(\"1.5\")   // => 1.5",
    ),
    // ── String static ──
    (
        "String.fromCharCode",
        "String static",
        "a string built from the given UTF-16 code units",
        "String.fromCharCode(65, 66)   // => 'AB'",
    ),
    // ── Array static ──
    (
        "Array.isArray",
        "Array static",
        "true if the value is an array",
        "Array.isArray([1, 2])   // => true",
    ),
    (
        "Array.from",
        "Array static",
        "build an array from an iterable or array-like",
        "Array.from(\"ab\")   // => [ 'a', 'b' ]",
    ),
    (
        "Array.of",
        "Array static",
        "build an array from the given arguments",
        "Array.of(1, 2, 3)   // => [ 1, 2, 3 ]",
    ),
    // ── Array method ──
    (
        "push",
        "Array method",
        "append items to the end; returns the new length",
        "const a = [1]; a.push(2); a   // => [ 1, 2 ]",
    ),
    (
        "pop",
        "Array method",
        "remove and return the last item",
        "[1, 2, 3].pop()   // => 3",
    ),
    (
        "shift",
        "Array method",
        "remove and return the first item",
        "[1, 2, 3].shift()   // => 1",
    ),
    (
        "unshift",
        "Array method",
        "prepend items; returns the new length",
        "const a = [2]; a.unshift(1); a   // => [ 1, 2 ]",
    ),
    (
        "map",
        "Array method",
        "a new array of the results of calling fn on each item",
        "[1, 2, 3].map(x => x * 2)   // => [ 2, 4, 6 ]",
    ),
    (
        "filter",
        "Array method",
        "a new array of the items for which fn is truthy",
        "[1, 2, 3, 4].filter(x => x % 2 === 0)   // => [ 2, 4 ]",
    ),
    (
        "forEach",
        "Array method",
        "call fn on each item for effect; returns undefined",
        "let s = 0; [1, 2, 3].forEach(x => s += x); s   // => 6",
    ),
    (
        "reduce",
        "Array method",
        "fold the array to a single value with an accumulator",
        "[1, 2, 3].reduce((a, b) => a + b, 0)   // => 6",
    ),
    (
        "join",
        "Array method",
        "concatenate items into a string with a separator",
        "[1, 2, 3].join(\"-\")   // => '1-2-3'",
    ),
    (
        "slice",
        "Array method",
        "a shallow copy of a [start, end) sub-range",
        "[1, 2, 3, 4].slice(1, 3)   // => [ 2, 3 ]",
    ),
    (
        "splice",
        "Array method",
        "remove/insert items in place; returns the removed items",
        "const a = [1, 2, 3]; a.splice(1, 1); a   // => [ 1, 3 ]",
    ),
    (
        "concat",
        "Array method",
        "a new array joining this array with more arrays/values",
        "[1].concat([2, 3])   // => [ 1, 2, 3 ]",
    ),
    (
        "indexOf",
        "Array method",
        "index of the first matching item, or -1",
        "[1, 2, 3].indexOf(2)   // => 1",
    ),
    (
        "lastIndexOf",
        "Array method",
        "index of the last matching item, or -1",
        "[1, 2, 1].lastIndexOf(1)   // => 2",
    ),
    (
        "includes",
        "Array method",
        "true if the array contains the value",
        "[1, 2, 3].includes(2)   // => true",
    ),
    (
        "find",
        "Array method",
        "the first item for which fn is truthy, else undefined",
        "[1, 2, 3].find(x => x > 1)   // => 2",
    ),
    (
        "findIndex",
        "Array method",
        "the index of the first item for which fn is truthy, else -1",
        "[1, 2, 3].findIndex(x => x > 1)   // => 1",
    ),
    (
        "some",
        "Array method",
        "true if fn is truthy for any item",
        "[1, 2, 3].some(x => x > 2)   // => true",
    ),
    (
        "every",
        "Array method",
        "true if fn is truthy for every item",
        "[1, 2, 3].every(x => x > 0)   // => true",
    ),
    (
        "reverse",
        "Array method",
        "reverse the array in place",
        "[1, 2, 3].reverse()   // => [ 3, 2, 1 ]",
    ),
    (
        "sort",
        "Array method",
        "sort in place (default: by string order)",
        "[3, 1, 2].sort()   // => [ 1, 2, 3 ]",
    ),
    (
        "flat",
        "Array method",
        "a new array with sub-arrays flattened one level (or by depth)",
        "[1, [2, [3]]].flat()   // => [ 1, 2, [ 3 ] ]",
    ),
    (
        "flatMap",
        "Array method",
        "map each item then flatten the result one level",
        "[1, 2].flatMap(x => [x, x])   // => [ 1, 1, 2, 2 ]",
    ),
    (
        "fill",
        "Array method",
        "overwrite a range with a value in place",
        "[1, 2, 3].fill(0)   // => [ 0, 0, 0 ]",
    ),
    (
        "at",
        "Array method",
        "the item at an index, allowing negative indexing",
        "[1, 2, 3].at(-1)   // => 3",
    ),
    // ── String method ──
    (
        "toUpperCase",
        "String method",
        "a copy with all cased characters uppercased",
        "\"abc\".toUpperCase()   // => 'ABC'",
    ),
    (
        "toLowerCase",
        "String method",
        "a copy with all cased characters lowercased",
        "\"ABC\".toLowerCase()   // => 'abc'",
    ),
    (
        "charAt",
        "String method",
        "the character at an index",
        "\"hi\".charAt(1)   // => 'i'",
    ),
    (
        "charCodeAt",
        "String method",
        "the UTF-16 code unit at an index",
        "\"A\".charCodeAt(0)   // => 65",
    ),
    (
        "codePointAt",
        "String method",
        "the Unicode code point at an index",
        "\"A\".codePointAt(0)   // => 65",
    ),
    (
        "slice",
        "String method",
        "a substring over a [start, end) range (negatives allowed)",
        "\"hello\".slice(1, 3)   // => 'el'",
    ),
    (
        "substring",
        "String method",
        "a substring over a [start, end) range (no negatives)",
        "\"hello\".substring(0, 2)   // => 'he'",
    ),
    (
        "split",
        "String method",
        "an array of substrings split on a separator",
        "\"a,b,c\".split(\",\")   // => [ 'a', 'b', 'c' ]",
    ),
    (
        "trim",
        "String method",
        "a copy with leading and trailing whitespace removed",
        "\"  hi  \".trim()   // => 'hi'",
    ),
    (
        "trimStart",
        "String method",
        "a copy with leading whitespace removed",
        "\"  hi\".trimStart()   // => 'hi'",
    ),
    (
        "trimEnd",
        "String method",
        "a copy with trailing whitespace removed",
        "\"hi  \".trimEnd()   // => 'hi'",
    ),
    (
        "replace",
        "String method",
        "a copy with the first match of a substring replaced",
        "\"aaa\".replace(\"a\", \"b\")   // => 'baa'",
    ),
    (
        "replaceAll",
        "String method",
        "a copy with every match of a substring replaced",
        "\"aaa\".replaceAll(\"a\", \"b\")   // => 'bbb'",
    ),
    (
        "repeat",
        "String method",
        "the string repeated n times",
        "\"ab\".repeat(3)   // => 'ababab'",
    ),
    (
        "startsWith",
        "String method",
        "true if the string starts with the prefix",
        "\"hello\".startsWith(\"he\")   // => true",
    ),
    (
        "endsWith",
        "String method",
        "true if the string ends with the suffix",
        "\"hello\".endsWith(\"lo\")   // => true",
    ),
    (
        "padStart",
        "String method",
        "pad on the left to a target length",
        "\"5\".padStart(3, \"0\")   // => '005'",
    ),
    (
        "padEnd",
        "String method",
        "pad on the right to a target length",
        "\"5\".padEnd(3, \"0\")   // => '500'",
    ),
    // ── Number method ──
    (
        "toFixed",
        "Number method",
        "a fixed-point string with n digits after the decimal point",
        "(3.14159).toFixed(2)   // => '3.14'",
    ),
    (
        "toPrecision",
        "Number method",
        "a string with n significant digits",
        "(123.456).toPrecision(4)   // => '123.5'",
    ),
    (
        "toString",
        "Number method",
        "the string form of a number in an optional radix",
        "(255).toString(16)   // => 'ff'",
    ),
];

/// The builtin corpus, exposed for offline doc generation (`gen-docs`) and any
/// editor tooling that wants the same (name, chapter, doc, example) rows.
pub fn corpus() -> &'static [(&'static str, &'static str, &'static str, &'static str)] {
    CORPUS
}
