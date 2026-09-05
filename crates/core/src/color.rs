use std::io::IsTerminal;

/// Color decision helpers. `owo-colors` only auto-disables on `NO_COLOR` via
/// its `if_supports_color` API, which is not used here, so we gate colors
/// ourselves for stdout/stderr.
fn env_override() -> Option<bool> {
    if let Some(v) = std::env::var_os("NO_COLOR") {
        if !v.is_empty() {
            return Some(false);
        }
    }
    if let Some(v) = std::env::var_os("FORCE_COLOR") {
        return Some(v != "0");
    }
    None
}

pub fn stdout_enabled() -> bool {
    env_override().unwrap_or_else(|| std::io::stdout().is_terminal())
}

pub fn stderr_enabled() -> bool {
    env_override().unwrap_or_else(|| std::io::stderr().is_terminal())
}

/// Removes ANSI SGR escape sequences (`ESC [ ... m`) from `s`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // Swallow the full CSI sequence up to its terminating byte.
        if chars.clone().next() == Some('[') {
            chars.next();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        }
    }
    out
}
