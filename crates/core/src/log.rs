use owo_colors::OwoColorize;
use std::fmt::Display;

pub fn info(msg: impl Display) {
    eprintln!("{} {}", "[+]".green().bold(), msg);
}

pub fn warn(msg: impl Display) {
    eprintln!("{} {}", "[!]".yellow().bold(), msg);
}

pub fn error(msg: impl Display) {
    eprintln!("{} {}", "[X]".red().bold(), msg);
}

pub fn die(msg: impl Display) -> ! {
    error(msg);
    std::process::exit(1);
}
