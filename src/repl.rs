//! Interactive REPL (`node --repl`, or `node` on a TTY).
//!
//! Keeps one persistent host across lines (module globals, `var`/`function`
//! declarations, classes and requires survive between prompts). A line whose
//! braces/parens/brackets are unbalanced accumulates continuation lines until the
//! delimiters close, so multi-line functions and objects can be entered.
//!
//! No startup banner is printed — the prompt appears immediately (house rule).

use nu_ansi_term::Color;
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

/// Run the REPL loop.
pub fn run() {
    crate::host::reset_host();
    let mut line_editor = Reedline::create();
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("> ".to_string()),
        DefaultPromptSegment::Empty,
    );

    loop {
        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(mut buffer)) => {
                if buffer.trim().is_empty() {
                    continue;
                }
                // Accumulate continuation lines while delimiters stay open.
                let cont_prompt = DefaultPrompt::new(
                    DefaultPromptSegment::Basic("... ".to_string()),
                    DefaultPromptSegment::Empty,
                );
                while unbalanced(&buffer) {
                    match line_editor.read_line(&cont_prompt) {
                        Ok(Signal::Success(more)) => {
                            buffer.push('\n');
                            buffer.push_str(&more);
                        }
                        _ => break,
                    }
                }
                run_line(&buffer);
            }
            Ok(Signal::CtrlC) => continue,
            Ok(Signal::CtrlD) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

/// True while the buffer has more open `{`/`(`/`[` than close (ignoring
/// delimiters inside string/template/char literals). A coarse check — good
/// enough to know whether to keep reading continuation lines.
fn unbalanced(s: &str) -> bool {
    let mut depth: i32 = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

fn run_line(src: &str) {
    match crate::compile(src) {
        Ok(prog) => match crate::run_compiled(prog) {
            Ok(_) => {}
            Err(e) => eprintln!("{}", Color::Red.paint(e)),
        },
        Err(e) => eprintln!("{}", Color::Red.paint(e)),
    }
}
