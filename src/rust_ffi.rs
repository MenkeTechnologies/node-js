//! JavaScript wiring for inline Rust FFI (`rust { ... }` blocks).
//!
//! The heavy lifting lives in fusevm: [`fusevm::RustSugar`] scans and rewrites
//! the block at the source level, and [`fusevm::ffi`] compiles/loads/marshals
//! it. This module only supplies the JS-flavored [`fusevm::RustSugar`] config
//! and the desugar entry the parser calls. The emitted `__rust_compile(...)`
//! call and every exported bareword are resolved in [`crate::host::call_named`].

use fusevm::RustSugar;

/// Emit the JS statement a `rust { ... }` block desugars to: a call to the
/// `__rust_compile` builtin carrying the base64-encoded block body and its line.
/// base64's alphabet (`A-Za-z0-9+/=`) needs no escaping inside the double-quoted
/// JS string literal.
fn emit(b64: &str, line: usize) -> String {
    format!("__rust_compile(\"{b64}\", {line})")
}

/// JavaScript desugar config: C-family braces with `//` and `/* */` comments.
/// `newline_boundary` is `true` so a top-level `rust { ... }` on its own line is
/// recognized — `rust {` is never valid JS otherwise, so this only ever matches
/// an intended FFI block. The desugar runs on raw source BEFORE lexing, so the
/// block is replaced in place by a `__rust_compile(...)` expression statement.
pub const SUGAR: RustSugar = RustSugar {
    keyword: "rust",
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    newline_boundary: true,
    emit,
};

/// Rewrite every top-level `rust { ... }` block in JS source into a
/// `__rust_compile(...)` call, before lexing. No-op when the source has no
/// `rust` token.
pub fn desugar(src: &str) -> String {
    SUGAR.desugar(src)
}

#[cfg(test)]
mod tests {
    #[test]
    fn desugars_top_level_block() {
        let src =
            "rust { pub extern \"C\" fn add(a: i64, b: i64) -> i64 { a + b } }\nconsole.log(add(2, 3))\n";
        let out = super::desugar(src);
        assert!(out.contains("__rust_compile("), "no builtin call: {out}");
        assert!(!out.contains("pub extern"), "Rust body leaked: {out}");
        assert!(
            out.contains("console.log(add(2, 3))"),
            "trailing code lost: {out}"
        );
    }

    #[test]
    fn leaves_ordinary_js_untouched() {
        let src = "const x = \"hi\".length;\nconsole.log(x);\n";
        assert_eq!(super::desugar(src), src);
    }
}
