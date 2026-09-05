use crate::color;
use owo_colors::OwoColorize;
use std::fmt::Display;

fn eprintln_styled(prefix: &str, styled_prefix: String, msg: impl Display) {
    if color::stderr_enabled() {
        eprintln!("{} {}", styled_prefix, msg);
    } else {
        eprintln!("{prefix} {}", msg);
    }
}

pub fn info(msg: impl Display) {
    eprintln_styled("[+]", "[+]".green().bold().to_string(), msg);
}

pub fn warn(msg: impl Display) {
    eprintln_styled("[!]", "[!]".yellow().bold().to_string(), msg);
}

pub fn error(msg: impl Display) {
    eprintln_styled("[X]", "[X]".red().bold().to_string(), msg);
}

pub fn die(msg: impl Display) -> ! {
    error(msg);
    std::process::exit(1);
}
