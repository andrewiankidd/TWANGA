//! Terminal UX primitives shared across TWANGA CLIs.
//!
//! Public surface: selection menus, refreshing displays, ANSI colour helpers,
//! Ctrl-C handling, the MOTD banner. The MOTD draws splash strings from
//! [`twanga_core::SPLASHES`] so the CLI banner and the eventual Tauri main
//! menu draw from one source.
//!
//! Cursor moves and line clears route through `crossterm`, which auto-enables
//! Windows VT processing on first use. After any `StatusLine::update` or
//! `MultiLineDisplay::render` call on Windows, raw ANSI escape codes (for
//! colour, etc.) in subsequent writes are interpreted too — so the `color`
//! helpers below are safe to embed in `update` payloads.

use anyhow::{anyhow, Result};
use crossterm::{
    cursor,
    execute,
    terminal::{Clear, ClearType},
};
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

const MAX_RETRIES: usize = 3;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Install a Ctrl-C handler that flips a process-wide "shutdown requested"
/// flag. Call once near the start of `main()`. Long-running loops should then
/// poll [`is_shutdown_requested`] and exit cleanly when it returns `true`.
///
/// This turns Ctrl-C into a graceful stop instead of the default abort, which
/// on Windows produces exit status `0xc000013a` (STATUS_CONTROL_C_EXIT) and
/// makes `cargo run` print a confusing "process didn't exit successfully"
/// error after every user-initiated stop.
pub fn install_ctrl_c_handler() -> Result<()> {
    ctrlc::set_handler(|| {
        SHUTDOWN.store(true, Ordering::SeqCst);
    })
    .map_err(|e| anyhow!("failed to install Ctrl-C handler: {e}"))
}

/// `true` once the Ctrl-C handler has fired. Poll in long-running loops to
/// decide when to break out and return cleanly.
pub fn is_shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

/// Spawn a background thread that reads lines from stdin and forwards them
/// (lower-cased, trimmed) through the returned receiver.
///
/// Useful for "type q + Enter to stop" UX in a CLI loop without enabling raw
/// mode. The thread runs until stdin closes or the receiver is dropped.
/// Lines are trimmed and lower-cased before sending, so callers can match on
/// `"q"`, `"quit"`, `""`, etc. without re-normalising.
pub fn spawn_line_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            if stdin.lock().read_line(&mut line).is_err() {
                return;
            }
            let trimmed = line.trim().to_lowercase();
            if tx.send(trimmed).is_err() {
                return;
            }
        }
    });
    rx
}

/// Prompt the user to pick one of `options`. Returns the 0-based index.
pub fn select(prompt: &str, options: &[&str]) -> Result<usize> {
    if options.is_empty() {
        return Err(anyhow!("select: no options provided"));
    }
    let stdin = io::stdin();
    let stderr = io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        return Err(anyhow!(
            "select: stdin or stderr is not a terminal — cannot prompt interactively"
        ));
    }

    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();

    for attempt in 0..MAX_RETRIES {
        writeln!(stderr_lock, "{prompt}")?;
        for (i, opt) in options.iter().enumerate() {
            writeln!(stderr_lock, "  {}) {opt}", i + 1)?;
        }
        write!(stderr_lock, "> ")?;
        stderr_lock.flush()?;

        line.clear();
        stdin_lock.read_line(&mut line)?;

        if let Some(idx) = parse_selection(&line, options) {
            return Ok(idx);
        }

        if attempt + 1 < MAX_RETRIES {
            writeln!(
                stderr_lock,
                "invalid choice: {input:?}. try a number 1..={max} or a name.",
                input = line.trim(),
                max = options.len()
            )?;
        }
    }
    Err(anyhow!("select: too many invalid attempts"))
}

/// Like [`select`], but empty input returns `default_idx`. The default is
/// marked with a `*` in the printed menu.
pub fn select_with_default(
    prompt: &str,
    options: &[&str],
    default_idx: usize,
) -> Result<usize> {
    if options.is_empty() {
        return Err(anyhow!("select_with_default: no options provided"));
    }
    if default_idx >= options.len() {
        return Err(anyhow!(
            "select_with_default: default_idx {default_idx} out of range (len={})",
            options.len()
        ));
    }
    let stdin = io::stdin();
    let stderr = io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        return Err(anyhow!(
            "select_with_default: stdin or stderr is not a terminal"
        ));
    }
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();

    for attempt in 0..MAX_RETRIES {
        writeln!(stderr_lock, "{prompt}")?;
        for (i, opt) in options.iter().enumerate() {
            let marker = if i == default_idx { "*" } else { " " };
            writeln!(stderr_lock, " {marker} {}) {opt}", i + 1)?;
        }
        write!(stderr_lock, "> ")?;
        stderr_lock.flush()?;

        line.clear();
        stdin_lock.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(default_idx);
        }
        if let Some(idx) = parse_selection(trimmed, options) {
            return Ok(idx);
        }
        if attempt + 1 < MAX_RETRIES {
            writeln!(
                stderr_lock,
                "invalid choice: {trimmed:?}. enter a number, a name, or just press enter for the default."
            )?;
        }
    }
    Err(anyhow!(
        "select_with_default: too many invalid attempts"
    ))
}

/// Prompt for a free-form value with a default. Empty input returns the default;
/// non-empty input is passed to `parse`. The default value is shown in
/// `[brackets]` next to the prompt.
pub fn prompt_parsed<T, F>(prompt: &str, default: T, parse: F) -> Result<T>
where
    T: std::fmt::Display + Clone,
    F: Fn(&str) -> std::result::Result<T, String>,
{
    let stdin = io::stdin();
    let stderr = io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        return Err(anyhow!(
            "prompt_parsed: stdin or stderr is not a terminal"
        ));
    }
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();

    for attempt in 0..MAX_RETRIES {
        write!(stderr_lock, "{prompt} [{default}]: ")?;
        stderr_lock.flush()?;
        line.clear();
        stdin_lock.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match parse(trimmed) {
            Ok(v) => return Ok(v),
            Err(msg) => {
                if attempt + 1 < MAX_RETRIES {
                    writeln!(stderr_lock, "invalid: {msg}")?;
                }
            }
        }
    }
    Err(anyhow!("prompt_parsed: too many invalid attempts"))
}

/// An in-place refreshing single-line status display. Writes to stderr.
/// Falls back to appending newlines when stderr isn't a terminal.
pub struct StatusLine {
    is_terminal: bool,
}

impl StatusLine {
    pub fn new() -> Self {
        Self {
            is_terminal: io::stderr().is_terminal(),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }

    /// Replace the current status line with `text`.
    pub fn update(&mut self, text: &str) -> io::Result<()> {
        let mut stderr = io::stderr();
        if self.is_terminal {
            execute!(
                stderr,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
            )?;
            write!(stderr, "{text}")?;
        } else {
            writeln!(stderr, "{text}")?;
        }
        stderr.flush()
    }

    /// Emit a newline so subsequent regular output starts on a fresh line.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.is_terminal {
            let mut stderr = io::stderr();
            writeln!(stderr)?;
            stderr.flush()?;
        }
        Ok(())
    }
}

impl Default for StatusLine {
    fn default() -> Self {
        Self::new()
    }
}

/// An in-place refreshing N-line block. Each `render(rows)` overwrites the
/// previously-rendered rows in place. `rows.len()` must equal `row_count`.
///
/// First `render()` prints rows normally (advancing cursor below the block);
/// subsequent calls move cursor up `row_count` lines, clear each line, and
/// rewrite. Falls back to appending plain rows when stderr isn't a terminal.
pub struct MultiLineDisplay {
    row_count: usize,
    initial_printed: bool,
    is_terminal: bool,
}

impl MultiLineDisplay {
    pub fn new(row_count: usize) -> Self {
        Self {
            row_count,
            initial_printed: false,
            is_terminal: io::stderr().is_terminal(),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }

    pub fn render(&mut self, rows: &[String]) -> io::Result<()> {
        debug_assert_eq!(rows.len(), self.row_count, "row count must match");
        let mut stderr = io::stderr();

        if !self.is_terminal {
            for row in rows {
                writeln!(stderr, "{row}")?;
            }
            return stderr.flush();
        }

        if self.initial_printed {
            execute!(stderr, cursor::MoveUp(self.row_count as u16))?;
        }

        for row in rows {
            execute!(
                stderr,
                cursor::MoveToColumn(0),
                Clear(ClearType::CurrentLine),
            )?;
            writeln!(stderr, "{row}")?;
        }

        self.initial_printed = true;
        stderr.flush()
    }
}

/// MOTD banner: ASCII-art logo + a random splash from the shared list.
///
/// `print_banner()` writes to stderr so it doesn't pollute pipes /
/// redirections from stdout. Each subcommand of an interactive CLI calls it
/// once at startup. Splash selection is seeded from the system clock so each
/// invocation generally gets a different splash.
pub mod motd {
    use std::io::{self, Write};

    const LOGO: &str = "\
████████ ██     ██  █████  ███    ██  ██████   █████
   ██    ██     ██ ██   ██ ████   ██ ██       ██   ██
   ██    ██  █  ██ ███████ ██ ██  ██ ██   ███ ███████
   ██    ██ ███ ██ ██   ██ ██  ██ ██ ██    ██ ██   ██
   ██     ███ ███  ██   ██ ██   ████  ██████  ██   ██";

    const BAR_WIDTH: usize = 64;

    /// Pick one splash from [`twanga_core::SPLASHES`] using the system clock as a seed.
    pub fn pick_splash() -> &'static str {
        let mut iter = twanga_core::splashes();
        let count = iter.clone().count();
        if count == 0 {
            return "TWANGA";
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let idx = (seed % count as u64) as usize;
        iter.nth(idx).unwrap_or("TWANGA")
    }

    /// Print the MOTD banner (logo + a random splash) to stderr.
    pub fn print_banner() -> io::Result<()> {
        let bar: String = "═".repeat(BAR_WIDTH);
        let splash = pick_splash();
        let mut stderr = io::stderr().lock();
        writeln!(stderr)?;
        writeln!(stderr, "{bar}")?;
        for line in LOGO.lines() {
            writeln!(stderr, "{line}")?;
        }
        writeln!(stderr, "{bar}")?;
        writeln!(stderr, "  {splash}")?;
        writeln!(stderr, "{bar}")?;
        writeln!(stderr)?;
        stderr.flush()
    }
}

/// ANSI colour helpers. Each function returns an empty string when `enabled`
/// is false, so callers can interpolate them unconditionally and still get
/// clean output in non-terminal contexts.
pub mod color {
    pub fn green(enabled: bool) -> &'static str {
        if enabled { "\x1b[32m" } else { "" }
    }
    pub fn yellow(enabled: bool) -> &'static str {
        if enabled { "\x1b[33m" } else { "" }
    }
    pub fn red(enabled: bool) -> &'static str {
        if enabled { "\x1b[31m" } else { "" }
    }
    pub fn dim(enabled: bool) -> &'static str {
        if enabled { "\x1b[2m" } else { "" }
    }
    pub fn reset(enabled: bool) -> &'static str {
        if enabled { "\x1b[0m" } else { "" }
    }
}

/// Parse a user input line against an option list. Pure helper for tests.
fn parse_selection(input: &str, options: &[&str]) -> Option<usize> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(n) = trimmed.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return Some(n - 1);
        }
        return None;
    }
    let lower = trimmed.to_lowercase();
    options.iter().position(|o| o.to_lowercase() == lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_selection_matches_number_in_range() {
        assert_eq!(parse_selection("2", &["a", "b", "c"]), Some(1));
    }

    #[test]
    fn parse_selection_matches_exact_name() {
        assert_eq!(
            parse_selection("standard-banjo", &["standard-guitar", "standard-banjo"]),
            Some(1)
        );
    }

    #[test]
    fn parse_selection_is_case_insensitive_for_names() {
        assert_eq!(parse_selection("BANJO", &["banjo", "uke"]), Some(0));
    }

    #[test]
    fn parse_selection_trims_whitespace_and_newlines() {
        assert_eq!(parse_selection("  2  \n", &["a", "b"]), Some(1));
    }

    #[test]
    fn parse_selection_returns_none_on_unknown_name() {
        assert_eq!(parse_selection("xyz", &["a", "b"]), None);
    }

    #[test]
    fn parse_selection_returns_none_when_number_out_of_range() {
        assert_eq!(parse_selection("5", &["a", "b"]), None);
    }

    #[test]
    fn parse_selection_zero_is_invalid() {
        assert_eq!(parse_selection("0", &["a", "b"]), None);
    }

    #[test]
    fn parse_selection_empty_is_invalid() {
        assert_eq!(parse_selection("", &["a", "b"]), None);
        assert_eq!(parse_selection("   \n", &["a", "b"]), None);
    }
}
