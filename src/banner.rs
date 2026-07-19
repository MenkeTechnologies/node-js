//! The REPL wordmark. Deliberately NOT printed on startup — the interactive
//! `node --repl` shows a bare prompt with no banner, version stripe, or welcome
//! message (house rule: prompt appears immediately, no startup chatter). Kept as
//! a reusable asset should an explicit, user-requested `--version`-style banner
//! ever be wanted.

use nu_ansi_term::Color;

/// ANSI-Shadow wordmark, matching the house cyberpunk style.
pub const WORDMARK: &str = r#"
███╗   ██╗ ██████╗ ██████╗ ███████╗
████╗  ██║██╔═══██╗██╔══██╗██╔════╝
██╔██╗ ██║██║   ██║██║  ██║█████╗
██║╚██╗██║██║   ██║██║  ██║██╔══╝
██║ ╚████║╚██████╔╝██████╔╝███████╗
╚═╝  ╚═══╝ ╚═════╝ ╚═════╝ ╚══════╝
"#;

/// Render the banner + a one-line subtitle. Not called on REPL startup; provided
/// for any explicit user-requested invocation.
pub fn print_banner() {
    let v = env!("CARGO_PKG_VERSION");
    println!("{}", Color::Cyan.bold().paint(WORDMARK));
    println!(
        "{} {}  {}",
        Color::Purple.bold().paint("node-js"),
        Color::White.paint(format!("v{v}")),
        Color::DarkGray.paint("JavaScript on fusevm — bytecode VM + Cranelift JIT")
    );
}
