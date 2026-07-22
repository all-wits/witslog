//! CLI "design tokens" — one severity/status → color+glyph map, reused by
//! every renderer (`get`, `query`), plus TTY-aware ANSI gating.
//!
//! Hand-rolled ANSI (no `anstream`/`anstyle` dependency): clap's own `color`
//! support only styles clap's `--help`/usage text, not application data, and
//! `anstream`/`anstyle` are not explicit workspace dependencies — adding one
//! just for this would be a new dep for a handful of escape codes. This
//! module is Windows-safe because modern Windows Terminal / conhost (Win10+)
//! interpret ANSI SGR codes natively; `should_colorize` never emits color
//! into a pipe or non-terminal fallback, and `--json` output never routes
//! through here (callers gate that before reaching these functions).

use std::io::IsTerminal;
use witslog_core::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// Resolves whether ANSI escapes should be emitted: `--color` wins outright
/// (`never`/`always`); `auto` (the default) additionally honors `NO_COLOR`
/// (https://no-color.org/) and only colorizes when stdout is a real TTY —
/// so piped/redirected output (`witslog get <id> | cat`) stays plain.
pub fn should_colorize(mode: ColorMode) -> bool {
    match mode {
        ColorMode::Never => false,
        ColorMode::Always => true,
        ColorMode::Auto => {
            std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
        }
    }
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const BRIGHT_RED: &str = "\x1b[91m";
const ON_RED: &str = "\x1b[97;41m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const GREEN: &str = "\x1b[32m";

/// Severity → (ANSI color, glyph). Single source of truth — the CLI's
/// "design tokens" — every renderer below reads from here so severity
/// coloring can't drift between `get` and `query`.
fn severity_style(sev: Severity) -> (&'static str, &'static str) {
    match sev {
        Severity::Trace => (DIM, "·"),
        Severity::Debug => (DIM, "○"),
        Severity::Info => (BLUE, "ℹ"),
        Severity::Warn => (YELLOW, "▲"),
        Severity::Error => (RED, "✖"),
        Severity::Critical => (BRIGHT_RED, "✖"),
        Severity::Fatal => (ON_RED, "✖"),
    }
}

/// Renders `<glyph> <severity>` (e.g. `✖ error`), colorized when `colorize`.
pub fn severity_chip(sev: Severity, colorize: bool) -> String {
    let (color, glyph) = severity_style(sev);
    if colorize {
        format!("{color}{glyph} {}{RESET}", sev.as_str())
    } else {
        format!("{glyph} {}", sev.as_str())
    }
}

/// Renders the resolved/unresolved status badge.
pub fn status_badge(resolved: bool, colorize: bool) -> String {
    let (color, text) = if resolved {
        (GREEN, "\u{2713} resolved")
    } else {
        (RED, "\u{25cf} unresolved")
    };
    if colorize {
        format!("{color}{text}{RESET}")
    } else {
        text.to_string()
    }
}

/// Dims secondary text (error_code/tags chips) when colorized.
pub fn dim(text: &str, colorize: bool) -> String {
    if colorize {
        format!("{DIM}{text}{RESET}")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_colorize_never_is_always_false() {
        assert!(!should_colorize(ColorMode::Never));
    }

    #[test]
    fn should_colorize_always_is_always_true() {
        assert!(should_colorize(ColorMode::Always));
    }

    #[test]
    fn chips_plain_when_not_colorized() {
        let s = severity_chip(Severity::Error, false);
        assert!(!s.contains('\x1b'));
        assert!(s.contains("error"));
    }

    #[test]
    fn chips_carry_ansi_when_colorized() {
        let s = severity_chip(Severity::Error, true);
        assert!(s.contains('\x1b'));
        assert!(s.contains("error"));
    }

    #[test]
    fn status_badge_plain_vs_colorized() {
        let plain = status_badge(false, false);
        let colored = status_badge(false, true);
        assert!(!plain.contains('\x1b'));
        assert!(colored.contains('\x1b'));
        assert!(plain.contains("unresolved"));
        assert!(colored.contains("unresolved"));
    }
}
