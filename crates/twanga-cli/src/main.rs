mod tunings;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use twanga_audio::{InputStream, OutputStream};
use twanga_core::{Capo, Frequency, MidiNote, PresetEntry, PresetString, TunedString, Tuning};
use twanga_dsp::{Tuner, TunerMode, TunerReading};
use twanga_synth::{exp_decay, sine};
use twanga_tabs::{
    TabEvent, TabRecorder,
    alphatex::{self, AlphaTexWriter, ParsedTab},
};
use twanga_tui::{MultiLineDisplay, StatusLine, color};

const READ_CHUNK: usize = 4096;
const IN_TUNE_CENTS: f32 = 5.0;
const CLOSE_CENTS: f32 = 20.0;

const DEFAULT_BPM: u32 = 120;
const DEFAULT_RESOLUTION: &str = "1/8";
const DEFAULT_BLOCK_WIDTH: usize = 32;

#[derive(Parser)]
#[command(name = "twanga", about = "TWANGA CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Live tuner — capture audio and show detected pitch vs the nearest target.
    /// Without `--tuning`, prompts to pick an instrument (or "no instrument" for a chromatic tuner).
    Tune {
        /// Tuning preset. Omit or pass `--tuning` with no value to be prompted;
        /// pass `--tuning <slug>` to skip the prompt.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        tuning: Option<String>,
        /// Capo, in semitones. `--capo 3` is a uniform capo, `--capo "0,2,2,2,2,2"`
        /// is per-string for drop-D-style setups. Omit or pass `--capo` with no
        /// value to be prompted; default 0 = no capo.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        capo: Option<String>,
    },
    /// Play back a `.alphatex` recording. Scrolling cursor view, optional
    /// metronome click on each beat, optional "wait" mode that pauses until
    /// you play the expected note.
    Play {
        /// Path to a `.alphatex` file.
        path: PathBuf,
        /// Re-tune the tab to a different instrument's tuning. Notes are
        /// transposed by absolute pitch — e.g. play a uke tab on banjo with
        /// `--tuning standard-banjo`. Notes outside the target instrument's
        /// playable range are silently dropped. Omit or pass `--tuning` with
        /// no value to be prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        tuning: Option<String>,
        /// Override the tempo from the file (BPM). Omit or pass `--bpm` with
        /// no value to keep the file's tempo (no prompt — there's already a
        /// sensible default from the file).
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        bpm: Option<String>,
        /// Disable the metronome click (default: on).
        #[arg(long)]
        no_metronome: bool,
        /// Wait for the user to play each note before advancing the cursor.
        #[arg(long)]
        wait: bool,
        /// Loop playback. Pass with no value to loop the entire file, or with
        /// `START:END` (0-indexed column range, end exclusive) to loop a
        /// section, e.g. `--loop 0:20` or `--loop 20:30`.
        #[arg(
            long = "loop",
            num_args = 0..=1,
            default_missing_value = "full",
            value_name = "START:END",
        )]
        loop_spec: Option<String>,
        /// Capo position applied to the tab's tuning before pitch comparison.
        /// Omit or pass `--capo` with no value to be prompted; default 0 = no
        /// capo. Falls back to whatever the file's `\subtitle` embedded if
        /// neither flag nor prompt provides one.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        capo: Option<String>,
    },
    /// Live tab recorder — capture played notes as horizontal ASCII tab notation.
    /// Any argument left unset triggers an interactive prompt (with the default
    /// pre-filled — just press enter to accept).
    Record {
        /// Tuning preset. Omit or pass `--tuning` with no value to be prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        tuning: Option<String>,
        /// Tempo for the time grid (BPM, 20–400). Omit or pass `--bpm` with no
        /// value to be prompted; default 120.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        bpm: Option<String>,
        /// Note value per column. Accepts `1/4`, `1/8`, `1/16`, `1/32`. Omit or
        /// pass `--resolution` with no value to be prompted; default `1/8`.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        resolution: Option<String>,
        /// Columns per scrolling block (4–200). Omit or pass `--block-width`
        /// with no value to be prompted; default 32.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        block_width: Option<String>,
        /// Capo position. `--capo 3` records as if every string is 3 semitones
        /// higher; logged frets are capo-relative. Omit or pass `--capo` with
        /// no value to be prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        capo: Option<String>,
        /// Disable the metronome click (default: on). Same flag shape as the
        /// playback equivalent — the recorder's metronome ticks on each beat
        /// boundary derived from the current resolution (e.g. 1/8 → every
        /// other column). Useful for keeping a steady tempo while playing.
        #[arg(long)]
        no_metronome: bool,
        /// Human-readable title for the recording — written to `\title` in the
        /// alphaTex header AND used to derive the filename
        /// (`<slug>-<unix-secs>.alphatex` if provided, `recording-<unix-secs>`
        /// otherwise). Omit or pass `--title` with no value to be prompted.
        /// Accept the blank default to keep the pre-title filename shape.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        title: Option<String>,
    },
    /// List available audio input devices.
    Devices,
    /// Convert a tab file from one format to another.
    Convert { input: String, output: String },
    /// Manage user-defined tunings stored at the platform config dir alongside
    /// the built-in presets.
    Tunings {
        #[command(subcommand)]
        action: TuningsAction,
    },
}

#[derive(Subcommand)]
enum TuningsAction {
    /// List all known tunings (built-in + user-defined).
    List,
    /// Show the path to the user-tunings config file.
    Path,
    /// Define a new tuning interactively and save it to the user config.
    Add,
}

// ──────────────────────────────────────────────────────────────────────────
// `twanga tunings` subcommand
// ──────────────────────────────────────────────────────────────────────────

fn run_tunings_list() -> Result<()> {
    let known = tunings::all_known_tunings();
    if known.is_empty() {
        eprintln!("(no tunings registered)");
        return Ok(());
    }
    for k in known {
        let tag = match k.origin {
            tunings::Origin::Builtin => "built-in",
            tunings::Origin::User => "user",
        };
        let pitches: Vec<String> = k
            .entry
            .strings
            .iter()
            .map(|s| MidiNote(s.midi).name())
            .collect();
        println!(
            "{slug:<22} {tag:<10} {name} — [{pitches}]",
            slug = k.slug,
            tag = tag,
            name = k.entry.name,
            pitches = pitches.join(" "),
        );
    }
    Ok(())
}

fn run_tunings_path() -> Result<()> {
    let path = tunings::user_tunings_path()
        .ok_or_else(|| anyhow!("no user config directory available on this platform"))?;
    println!("{}", path.display());
    if path.exists() {
        eprintln!("(file exists)");
    } else {
        eprintln!("(file does not exist yet — run `twanga tunings add` to create it)");
    }
    Ok(())
}

fn run_tunings_add() -> Result<()> {
    eprintln!("Define a new tuning. Strings are entered in string-number order");
    eprintln!("(string 1 first), NOT pitch order — this matters for the banjo");
    eprintln!("5th-string drone and the ukulele's reentrant high G. For reentrant");
    eprintln!("or drone labels, hand-edit the TOML afterwards (see `tunings path`).");
    eprintln!();

    let string_count: usize = twanga_tui::prompt_parsed("How many strings?", 6_usize, |s| {
        s.parse::<usize>().map_err(|e| e.to_string()).and_then(|n| {
            if (1..=20).contains(&n) {
                Ok(n)
            } else {
                Err("expected 1-20".into())
            }
        })
    })?;

    let mut strings: Vec<PresetString> = Vec::with_capacity(string_count);
    for i in 1..=string_count {
        let pitch = prompt_required_note(&format!("String {i} open pitch (e.g. A4, C#3)"))?;
        strings.push(PresetString {
            name: pitch.name(),
            midi: pitch.0,
        });
    }

    let default_name = "My Tuning".to_string();
    let name: String =
        twanga_tui::prompt_parsed("Display name", default_name, |s| Ok::<_, String>(s.into()))?;

    let default_slug = slugify(&name);
    let slug: String = twanga_tui::prompt_parsed("Slug (kebab-case)", default_slug, |s| {
        validate_slug(s).map(|_| s.to_string())
    })?;

    let entry = PresetEntry {
        slug: slug.clone(),
        name: name.clone(),
        strings,
    };

    let path = tunings::add_user_tuning(entry)?;
    eprintln!();
    eprintln!("Saved '{slug}' to {}", path.display());
    eprintln!("It will appear in the tune/record/play menus from now on.");
    Ok(())
}

/// Prompt for a required MIDI note (no default — empty input retries).
fn prompt_required_note(prompt: &str) -> Result<MidiNote> {
    use std::io::{BufRead, IsTerminal, Write};
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        return Err(anyhow!(
            "prompt_required_note: stdin or stderr is not a terminal"
        ));
    }
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();
    for _ in 0..5 {
        write!(stderr_lock, "{prompt}: ")?;
        stderr_lock.flush()?;
        line.clear();
        stdin_lock.read_line(&mut line)?;
        let trimmed = line.trim();
        if let Some(m) = MidiNote::from_name(trimmed) {
            return Ok(m);
        }
        writeln!(stderr_lock, "invalid note name (try A4, C#3, etc.)")?;
    }
    Err(anyhow!("too many invalid note names"))
}

/// Lowercase, replace non-alphanumeric runs with single hyphens, trim hyphens.
/// `"Tenor Banjo (CGDA)"` → `"tenor-banjo-cgda"`.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_hyphen = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("custom-tuning");
    }
    out
}

fn validate_slug(s: &str) -> std::result::Result<(), String> {
    if s.is_empty() {
        return Err("slug cannot be empty".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("slug must be lowercase ASCII letters, digits, and hyphens".into());
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err("slug cannot start or end with a hyphen".into());
    }
    Ok(())
}

/// Slug list for menus — built-in presets first, then user-defined.
fn known_slugs() -> Vec<String> {
    tunings::all_known_tunings()
        .into_iter()
        .map(|k| k.slug)
        .collect()
}

/// Resolve a slug to a `Tuning` via the merged registry. Returns `None` if
/// the slug isn't in either the built-in or the user file.
fn lookup_tuning(slug: &str) -> Option<Tuning> {
    tunings::lookup(slug).map(|k| k.to_tuning())
}

/// CLI mode menu for `tune`: index 0 is chromatic; the rest are slugs from
/// the merged registry (built-in + user-defined).
fn tune_menu_options() -> Vec<String> {
    let slugs = known_slugs();
    let mut v = Vec::with_capacity(1 + slugs.len());
    v.push("(no instrument — chromatic tuner)".to_string());
    v.extend(slugs);
    v
}

/// Helper: treat an `Option<String>` as "explicit value present" only when
/// the string is non-empty. With `num_args = 0..=1` + `default_missing_value
/// = ""`, a bare `--flag` (no value) shows up as `Some("")` from clap — both
/// `None` and `Some("")` mean "prompt me." Lets the same flag work as
/// `--flag` (prompt), `--flag value` (use directly), or absent (prompt /
/// default depending on subcommand).
fn flag_value(arg: &Option<String>) -> Option<&str> {
    arg.as_deref().filter(|s| !s.is_empty())
}

fn resolve_mode(arg: Option<String>) -> Result<TunerMode> {
    if let Some(name) = flag_value(&arg) {
        let tuning = lookup_tuning(name).ok_or_else(|| {
            anyhow!(
                "unknown preset '{name}'. options: {}",
                known_slugs().join(", ")
            )
        })?;
        return Ok(TunerMode::Strings(tuning));
    }

    let options = tune_menu_options();
    let refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let idx = twanga_tui::select_with_hint(
        "Choose a tuning:",
        &refs,
        Some("tip: define a custom tuning with `twanga tunings add`"),
    )?;
    if idx == 0 {
        Ok(TunerMode::Chromatic)
    } else {
        let slug = &options[idx];
        let tuning = lookup_tuning(slug)
            .ok_or_else(|| anyhow!("preset registry desync; report this bug"))?;
        Ok(TunerMode::Strings(tuning))
    }
}

/// Resolve a capo. If `arg` is a non-empty value, parse it against `tuning`'s
/// string count. Otherwise (absent or bare `--capo`), prompt for a uniform
/// integer (default 0). The interactive prompt only supports uniform capos;
/// partial capos must come from the `--capo "0,2,2,..."` flag value.
fn resolve_capo(arg: Option<String>, tuning: &Tuning) -> Result<Capo> {
    let n_strings = tuning.strings.len();
    if let Some(spec) = flag_value(&arg) {
        return Capo::parse(spec, n_strings).map_err(|e| anyhow!("{e}"));
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(Capo::none(n_strings));
    }
    let n: i32 = twanga_tui::prompt_parsed("Capo position (semitones)", 0_i32, |s| {
        s.trim().parse::<i32>().map_err(|e| e.to_string())
    })?;
    Ok(Capo::uniform(n_strings, n))
}

fn resolve_tuning(arg: Option<String>) -> Result<Tuning> {
    if let Some(name) = flag_value(&arg) {
        return lookup_tuning(name).ok_or_else(|| {
            anyhow!(
                "unknown preset '{name}'. options: {}",
                known_slugs().join(", ")
            )
        });
    }
    let slugs = known_slugs();
    let refs: Vec<&str> = slugs.iter().map(|s| s.as_str()).collect();
    let idx = twanga_tui::select_with_hint(
        "Choose a tuning to record against:",
        &refs,
        Some("tip: define a custom tuning with `twanga tunings add`"),
    )?;
    lookup_tuning(&slugs[idx]).ok_or_else(|| anyhow!("preset registry desync; report this bug"))
}

/// If `arg` is provided, return as-is. Otherwise prompt with the file's own
/// tuning as the default first option (press enter to accept). Returns `None`
/// for "use the file's tuning unchanged" or `Some(preset)` for transposition.
///
/// Non-TTY callers skip the prompt and get `None`, matching the previous
/// behaviour where `play` without `--tuning` just used the file's tuning.
fn resolve_play_tuning(arg: Option<String>, tab: &ParsedTab) -> Result<Option<String>> {
    if let Some(name) = flag_value(&arg) {
        return Ok(Some(name.to_string()));
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(None);
    }
    let as_recorded = format!("(as recorded in file: {})", tab.tuning_names.join(" "));
    let slugs = known_slugs();
    let mut owned: Vec<String> = Vec::with_capacity(1 + slugs.len());
    owned.push(as_recorded);
    owned.extend(slugs.iter().cloned());
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let idx = twanga_tui::select_with_default_and_hint(
        "Choose a tuning for playback:",
        &refs,
        0,
        Some("tip: define a custom tuning with `twanga tunings add`"),
    )?;
    if idx == 0 {
        Ok(None)
    } else {
        Ok(Some(slugs[idx - 1].clone()))
    }
}

/// Resolve a BPM. If `arg` is a non-empty value, parse and validate. Otherwise
/// (absent or bare `--bpm`) prompt with the default.
fn resolve_bpm(arg: Option<String>) -> Result<u32> {
    if let Some(s) = flag_value(&arg) {
        let n: u32 = s
            .parse()
            .map_err(|_| anyhow!("invalid BPM '{s}' (expected an integer)"))?;
        validate_bpm(n)?;
        return Ok(n);
    }
    twanga_tui::prompt_parsed("Tempo (BPM)", DEFAULT_BPM, |s| {
        let n: u32 = s
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        validate_bpm(n).map_err(|e| e.to_string())?;
        Ok(n)
    })
}

fn validate_bpm(n: u32) -> Result<()> {
    if (20..=400).contains(&n) {
        Ok(())
    } else {
        Err(anyhow!("BPM out of range (20-400): {n}"))
    }
}

/// Resolve a `play --bpm` override. Like `resolve_bpm` but the "absent" case
/// returns `None` (let the file's tempo win) instead of prompting — no point
/// asking the user to retype a value that's already in the file.
fn resolve_bpm_override(arg: Option<String>) -> Result<Option<u32>> {
    if let Some(s) = flag_value(&arg) {
        let n: u32 = s
            .parse()
            .map_err(|_| anyhow!("invalid BPM '{s}' (expected an integer)"))?;
        validate_bpm(n)?;
        return Ok(Some(n));
    }
    Ok(None)
}

/// If `arg` is a non-empty value, parse it; otherwise prompt from the fixed list.
fn resolve_resolution(arg: Option<String>) -> Result<u32> {
    if let Some(r) = flag_value(&arg) {
        return parse_resolution(r);
    }
    const LABELS: &[&str] = &["1/4", "1/8", "1/16", "1/32"];
    const DENOMS: &[u32] = &[4, 8, 16, 32];
    let default_idx = LABELS
        .iter()
        .position(|l| *l == DEFAULT_RESOLUTION)
        .unwrap_or(1);
    let idx = twanga_tui::select_with_default("Resolution:", LABELS, default_idx)?;
    Ok(DENOMS[idx])
}

/// If `arg` is a non-empty value, parse and validate; otherwise prompt with
/// the default.
fn resolve_block_width(arg: Option<String>) -> Result<usize> {
    if let Some(s) = flag_value(&arg) {
        let n: usize = s
            .parse()
            .map_err(|_| anyhow!("invalid block width '{s}' (expected an integer)"))?;
        validate_block_width(n)?;
        return Ok(n);
    }
    twanga_tui::prompt_parsed(
        "Block width (columns per scrolling block)",
        DEFAULT_BLOCK_WIDTH,
        |s| {
            let n: usize = s
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            validate_block_width(n).map_err(|e| e.to_string())?;
            Ok(n)
        },
    )
}

/// Resolve `--title` for `record`. Three-form pattern: explicit value
/// passes through, bare flag prompts (default blank → no title), omission
/// also defers to a prompt. Blank input is preserved as `None` so the
/// recording lands at `recording-<unix-secs>.alphatex` like the pre-title
/// era. Any non-blank value flows into `\title` and into the filename slug.
fn resolve_title(arg: Option<String>) -> Result<Option<String>> {
    if let Some(s) = arg {
        // `flag_value` strips bare-form empties; we handle blanks below, so
        // bypass it here and treat the raw arg as the user's input.
        let trimmed = s.trim();
        return Ok(if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        });
    }
    // No flag passed: prompt with a blank default. User can press enter to
    // skip — the recording falls back to the pre-title filename shape.
    let raw: String = twanga_tui::prompt_parsed("Title (blank = no title)", String::new(), |s| {
        Ok::<_, String>(s.trim().to_string())
    })?;
    Ok(if raw.is_empty() { None } else { Some(raw) })
}

fn validate_block_width(n: usize) -> Result<()> {
    if (4..=200).contains(&n) {
        Ok(())
    } else {
        Err(anyhow!("block width out of range (4-200): {n}"))
    }
}

/// Parse a `1/N` resolution into the integer N. Supports the conventional set.
fn parse_resolution(s: &str) -> Result<u32> {
    let mut parts = s.split('/');
    let num = parts
        .next()
        .ok_or_else(|| anyhow!("invalid resolution '{s}'"))?;
    let denom = parts
        .next()
        .ok_or_else(|| anyhow!("invalid resolution '{s}' (expected `1/N`)"))?;
    if parts.next().is_some() {
        return Err(anyhow!("invalid resolution '{s}' (expected `1/N`)"));
    }
    if num != "1" {
        return Err(anyhow!("resolution must be of the form `1/N` (got '{s}')"));
    }
    let n: u32 = denom
        .parse()
        .map_err(|_| anyhow!("invalid resolution denominator '{denom}'"))?;
    if !matches!(n, 4 | 8 | 16 | 32) {
        return Err(anyhow!(
            "supported resolutions: 1/4, 1/8, 1/16, 1/32 (got '{s}')"
        ));
    }
    Ok(n)
}

fn action_for(cents: f32) -> &'static str {
    if cents.abs() < IN_TUNE_CENTS {
        "Tuned!"
    } else if cents > 0.0 {
        "Tune Down!"
    } else {
        "Tune Up!"
    }
}

fn cents_color(cents: f32, enabled: bool) -> &'static str {
    let abs = cents.abs();
    if abs < IN_TUNE_CENTS {
        color::green(enabled)
    } else if abs < CLOSE_CENTS {
        color::yellow(enabled)
    } else {
        color::red(enabled)
    }
}

fn format_chromatic_row(r: &TunerReading, use_color: bool) -> String {
    let c = cents_color(r.cents, use_color);
    let reset = color::reset(use_color);
    format!(
        "{:<6} | current: {:>9.2} Hz | target: {:>9.2} Hz | {c}{} ({:+.1} cents){reset}",
        r.label,
        r.detected.hz(),
        r.target.hz(),
        action_for(r.cents),
        r.cents,
    )
}

fn format_string_row(
    name: &str,
    current: Option<(f32, f32)>,
    target_hz: f32,
    use_color: bool,
) -> String {
    let reset = color::reset(use_color);
    let dim = color::dim(use_color);
    match current {
        None => format!(
            "{:<16} | current: {:>12} | target: {:>9.2} Hz | {dim}(play to detect){reset}",
            name, "—", target_hz,
        ),
        Some((detected, cents)) => {
            let c = cents_color(cents, use_color);
            format!(
                "{:<16} | current: {:>9.2} Hz | target: {:>9.2} Hz | {c}{} ({:+.1} cents){reset}",
                name,
                detected,
                target_hz,
                action_for(cents),
                cents,
            )
        }
    }
}

/// Returns true if `input` is one of the recognised "quit" line-reader inputs.
fn is_quit_input(input: &str) -> bool {
    matches!(input, "q" | "quit" | "exit")
}

fn run_chromatic(mut tuner: Tuner, mut stream: InputStream) -> Result<()> {
    let mut status = StatusLine::new();
    let use_color = status.is_terminal();
    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();

    status.update("(play any note to begin)")?;

    loop {
        if twanga_tui::is_shutdown_requested() {
            status.finish()?;
            return Ok(());
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                status.finish()?;
                return Ok(());
            }
        }
        let n = stream.read(&mut buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            for r in tuner.take_readings() {
                status.update(&format_chromatic_row(&r, use_color))?;
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn run_strings(mut tuner: Tuner, mut stream: InputStream, strings: Vec<TunedString>) -> Result<()> {
    let mut display = MultiLineDisplay::new(strings.len());
    let use_color = display.is_terminal();
    let mut row_states: Vec<Option<(f32, f32)>> = vec![None; strings.len()];

    let make_rows = |states: &[Option<(f32, f32)>], use_color: bool| -> Vec<String> {
        strings
            .iter()
            .zip(states.iter())
            .map(|(s, state)| {
                format_string_row(&s.name, *state, s.open.to_frequency().hz(), use_color)
            })
            .collect()
    };

    display.render(&make_rows(&row_states, use_color))?;

    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();

    loop {
        if twanga_tui::is_shutdown_requested() {
            return Ok(());
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                return Ok(());
            }
        }
        let n = stream.read(&mut buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            let mut changed = false;
            for r in tuner.take_readings() {
                if let Some(idx) = strings.iter().position(|s| s.name == r.label) {
                    row_states[idx] = Some((r.detected.hz(), r.cents));
                    changed = true;
                }
            }
            if changed {
                display.render(&make_rows(&row_states, use_color))?;
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn run_tuner(mode: TunerMode) -> Result<()> {
    let stream = InputStream::open()?;
    let sample_rate = stream.sample_rate;
    let channels = stream.channels;

    let header_label = match &mode {
        TunerMode::Chromatic => "(chromatic — guesses the nearest note)".to_string(),
        TunerMode::Strings(t) => format!("{} ({} strings)", t.name, t.strings.len()),
    };

    eprintln!("Tuning: {header_label}");
    eprintln!("Device: {}", stream.device_name);
    eprintln!("Audio:  {sample_rate} Hz, {channels} channel(s)");
    eprintln!();
    eprintln!("─────────────────────────────────────────────────");
    eprintln!("  Controls: type 'q' + Enter to stop  (or Ctrl-C)");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let strings = match &mode {
        TunerMode::Chromatic => None,
        TunerMode::Strings(t) => Some(t.strings.clone()),
    };

    let tuner = Tuner::new(mode, sample_rate);
    match strings {
        None => run_chromatic(tuner, stream),
        Some(s) => run_strings(tuner, stream, s),
    }
}

/// Highest fret the recorder will accept on any string. Anything past this is
/// treated as garbage / out-of-range and silently dropped.
const MAX_FRET: u8 = 20;

/// Folder (relative to CWD) where `twanga record` writes its output files.
const RECORDINGS_DIR: &str = "recordings";

/// Open an alphaTex recording file for `twanga record`. Writes the header
/// against the BASE tuning + capo (the writer encodes the capo into the
/// `\subtitle` line so the file round-trips through other tools), embeds
/// the optional `title` into `\title`, and returns the path + a streaming
/// writer ready for per-column writes.
///
/// Filename:
/// - `title` provided → `<slug>-<unix-secs>.alphatex` (timestamp suffix
///   guarantees uniqueness even if the user records the same song twice).
/// - `title` blank/`None` → `recording-<unix-secs>.alphatex` (the original
///   pre-title-feature shape, so older tooling that globs for
///   `recording-*.alphatex` keeps working).
fn open_recording_file(
    base_tuning: &Tuning,
    capo: &Capo,
    bpm: u32,
    resolution_denom: u32,
    title: Option<&str>,
) -> Result<(PathBuf, AlphaTexWriter<BufWriter<File>>)> {
    let dir = PathBuf::from(RECORDINGS_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create '{}' directory", dir.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("{}-{secs}", slugify(t)),
        None => format!("recording-{secs}"),
    };
    let path = dir.join(format!("{stem}.alphatex"));
    let file =
        File::create(&path).with_context(|| format!("failed to create '{}'", path.display()))?;
    let writer = AlphaTexWriter::new(
        BufWriter::new(file),
        base_tuning,
        capo,
        bpm,
        resolution_denom,
        title,
    )
    .with_context(|| format!("failed to write alphaTex header to '{}'", path.display()))?;
    Ok((path, writer))
}

fn finalize_recording(
    writer: &mut AlphaTexWriter<BufWriter<File>>,
    path: &Path,
    reason: &str,
) -> Result<()> {
    writer.finalize().ok();
    eprintln!();
    eprintln!("{reason}");
    eprintln!("Saved to: {}", path.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_recorder(
    base_tuning: Tuning,
    capo: Capo,
    bpm: u32,
    resolution_denom: u32,
    block_width: usize,
    metronome: bool,
    title: Option<String>,
) -> Result<()> {
    let mut stream = InputStream::open()?;
    let sample_rate = stream.sample_rate;
    let ms_per_col = 240_000 / (bpm * resolution_denom);
    // Same beat-boundary derivation `run_playback` uses: at 1/8 resolution,
    // every other column is a beat; at 1/16, every fourth; etc.
    let cols_per_beat = (resolution_denom as usize / 4).max(1);

    // Open an output stream only if the user wants a metronome — saves a
    // device acquisition + a pre-computed click buffer when the user is
    // recording to a backing track or doesn't want the click in the mic.
    let mut output = if metronome {
        Some(OutputStream::open()?)
    } else {
        None
    };
    let click = output.as_ref().map(|o| metronome_click(o.sample_rate));

    // Effective tuning = base + capo. Used for everything pitch-related at
    // runtime (display, fret matching). The alphaTex header still gets the
    // BASE tuning + capo via `open_recording_file`, so the file round-trips
    // through alphaTab and through `twanga play --capo N`.
    let effective = capo.apply(&base_tuning).map_err(|e| anyhow!("{e}"))?;
    let tuning_name = effective.name.clone();
    let string_count = effective.strings.len();

    let mut recorder = TabRecorder::new(&effective, sample_rate, ms_per_col, block_width);
    // Chromatic mode: gives us raw detected frequencies (silence-gated) without
    // imposing the tuner's ±7 semitone string-distance gate, which would clip
    // high-fret recording. We do our own fret-aware string match below.
    let mut tuner = Tuner::new(TunerMode::Chromatic, sample_rate);

    let (recording_path, mut recording_writer) =
        open_recording_file(&base_tuning, &capo, bpm, resolution_denom, title.as_deref())?;

    if let Some(t) = &title {
        eprintln!("Title:      {t}");
    }
    eprintln!("Tuning:     {tuning_name} ({string_count} strings)");
    if !capo.is_none() {
        if let Some(n) = capo.is_uniform() {
            eprintln!("Capo:       {n} (uniform)");
        } else {
            eprintln!("Capo:       [{}] (partial)", capo.serialize());
        }
    }
    eprintln!("Device:     {}", stream.device_name);
    eprintln!("Audio:      {sample_rate} Hz");
    eprintln!("Tempo:      {bpm} BPM, 1/{resolution_denom} notes ({ms_per_col} ms/col)",);
    eprintln!(
        "Block:      {block_width} cols ({} ms wide)",
        block_width as u32 * ms_per_col,
    );
    eprintln!("Metronome:  {}", if metronome { "on" } else { "off" });
    eprintln!("Saving to:  {}", recording_path.display());
    eprintln!();
    eprintln!("─────────────────────────────────────────────────");
    eprintln!("  Controls: type 'q' + Enter to stop  (or Ctrl-C)");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let n_rows = recorder.string_count();
    // +1 row for the duration / column-count status appended below the tab.
    // Each block recreates the display so the status sits at the bottom of
    // whatever block is currently growing.
    let mut display: Option<MultiLineDisplay> = None;
    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();
    let mut total_samples: u64 = 0;
    let mut total_columns: u64 = 0;
    // Aggregate count of detected pitches that no string + fret combo could
    // reach on the active (post-capo) tuning. Per the parity audit's
    // "couldn't fit on fretboard" item — silent drops were the previous
    // behaviour. Per-frame logging is too noisy; aggregate is enough.
    let mut total_dropped: u64 = 0;

    loop {
        if twanga_tui::is_shutdown_requested() {
            return finalize_recording(
                &mut recording_writer,
                &recording_path,
                "Recording stopped (Ctrl-C).",
            );
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                return finalize_recording(
                    &mut recording_writer,
                    &recording_path,
                    "Recording stopped.",
                );
            }
        }
        let n = stream.read(&mut buf);
        if n > 0 {
            total_samples += n as u64;
            tuner.feed(&buf[..n]);
            for r in tuner.take_readings() {
                match effective.match_to_fret(r.detected, MAX_FRET) {
                    Some(m) => recorder.record_hit(m.string_idx, m.fret),
                    None => total_dropped += 1,
                }
            }
            for event in recorder.advance(n) {
                let (rows, column_marks, is_block_complete) = match &event {
                    TabEvent::ColumnTick { rows, column_marks } => (rows, column_marks, false),
                    TabEvent::BlockComplete { rows, column_marks } => (rows, column_marks, true),
                };

                // Every committed column gets written to alphaTex.
                recording_writer.write_column(column_marks)?;
                total_columns += 1;

                // Metronome click on each beat boundary. `total_columns` is
                // 1-based after the increment above, so col 1 (the first
                // committed column) is treated as the downbeat — same
                // convention `run_playback` uses.
                if (total_columns - 1) % cols_per_beat as u64 == 0
                    && let (Some(out), Some(click)) = (output.as_mut(), click.as_ref())
                {
                    out.write(click);
                }

                // Display: refresh in place for ColumnTick, finalize for BlockComplete.
                // We render n_rows tab strings + 1 status line below, so the
                // multi-line display tracks n_rows + 1 rows.
                let elapsed_secs = total_samples / sample_rate.max(1) as u64;
                let elapsed = format_mmss(elapsed_secs);
                let mut rows_with_status: Vec<String> = rows.clone();
                let mut status = format!("  {elapsed} | {total_columns} cols");
                if total_dropped > 0 {
                    status.push_str(&format!(
                        " | {total_dropped} dropped (out of fretboard range)"
                    ));
                }
                rows_with_status.push(status);
                let d = display.get_or_insert_with(|| MultiLineDisplay::new(n_rows + 1));
                d.render(&rows_with_status)?;
                if is_block_complete {
                    // After render(), cursor sits on the line below the
                    // block — a blank println pushes the next block one
                    // row further down so blocks are visually separated.
                    println!();
                    display = None;
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn main() -> Result<()> {
    twanga_tui::install_ctrl_c_handler()?;
    let cli = Cli::parse();
    // Banner once at the top — every subcommand gets it. It goes to stderr so
    // piped stdout (e.g. `twanga devices | grep USB`, `twanga tunings path`
    // inside a `$()`) stays clean; interactive users see a consistent splash.
    twanga_tui::motd::print_banner()?;
    match cli.command {
        Command::Tune { tuning, capo } => {
            let mode = resolve_mode(tuning)?;
            let mode = match mode {
                TunerMode::Chromatic => TunerMode::Chromatic,
                TunerMode::Strings(t) => {
                    let c = resolve_capo(capo, &t)?;
                    let effective = c.apply(&t).map_err(|e| anyhow!("{e}"))?;
                    TunerMode::Strings(effective)
                }
            };
            run_tuner(mode)?;
        }
        Command::Record {
            tuning,
            bpm,
            resolution,
            block_width,
            capo,
            no_metronome,
            title,
        } => {
            let t = resolve_tuning(tuning)?;
            let c = resolve_capo(capo, &t)?;
            let bpm = resolve_bpm(bpm)?;
            let denom = resolve_resolution(resolution)?;
            let bw = resolve_block_width(block_width)?;
            let title = resolve_title(title)?;
            run_recorder(t, c, bpm, denom, bw, !no_metronome, title)?;
        }
        Command::Play {
            path,
            tuning,
            bpm,
            no_metronome,
            wait,
            loop_spec,
            capo,
        } => {
            // Parse first so the tuning prompt can show what's in the file.
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            let parsed =
                alphatex::parse(&content).map_err(|e| anyhow!("failed to parse alphaTex: {e}"))?;
            if parsed.columns.is_empty() {
                return Err(anyhow!("'{}' has no notes to play", path.display()));
            }
            let tuning = resolve_play_tuning(tuning, &parsed)?;
            let bpm = resolve_bpm_override(bpm)?;
            run_playback(
                path,
                parsed,
                tuning,
                bpm,
                !no_metronome,
                wait,
                loop_spec,
                capo,
            )?;
        }
        Command::Devices => {
            for name in twanga_audio::list_input_devices()? {
                println!("{name}");
            }
        }
        Command::Convert { input, output } => {
            println!("convert: not yet implemented ({input} -> {output})");
        }
        Command::Tunings { action } => match action {
            TuningsAction::List => run_tunings_list()?,
            TuningsAction::Path => run_tunings_path()?,
            TuningsAction::Add => run_tunings_add()?,
        },
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// Playback
// ──────────────────────────────────────────────────────────────────────────

/// Width (in tab columns) of the scrolling playback view.
const PLAYBACK_WINDOW_COLS: usize = 24;
/// Tolerance for wait-mode note matching, in cents.
const WAIT_MATCH_CENTS: f32 = 50.0;

// 8 args is one over clippy's default ceiling. The arguments are
// independent user-CLI-flag values aggregated for the playback flow;
// bundling them into a struct just to satisfy the lint is more
// ceremony than clarity at this size.
#[allow(clippy::too_many_arguments)]
fn run_playback(
    path: PathBuf,
    parsed: ParsedTab,
    tuning_override: Option<String>,
    bpm_override: Option<u32>,
    metronome: bool,
    wait: bool,
    loop_spec: Option<String>,
    capo_spec: Option<String>,
) -> Result<()> {
    // Transpose if --tuning was provided (either explicitly via flag or via
    // the prompt). The transposed tab carries the target tuning's names in
    // its header, so downstream code (wait mode, display) sees one consistent
    // tuning throughout. We use the *with-report* variant so out-of-range
    // notes get surfaced to the user up front rather than silently
    // disappearing — they get a "Skipped notes" preamble before the cursor
    // starts and can hit `q` if it's worse than they expected.
    let (tab, dropped) = if let Some(name) = tuning_override.as_deref() {
        let target = lookup_tuning(name).ok_or_else(|| {
            anyhow!(
                "unknown tuning preset '{name}'. options: {}",
                known_slugs().join(", ")
            )
        })?;
        parsed.transpose_to_with_report(&target, MAX_FRET)
    } else {
        (parsed, Vec::new())
    };

    let (loop_start, loop_end, repeat) = parse_loop_spec(loop_spec.as_deref(), tab.columns.len())?;

    let bpm = bpm_override.unwrap_or(tab.tempo);
    let resolution_denom = tab.columns[0].duration_denom;
    let ms_per_col = 240_000 / (bpm * resolution_denom);
    let cols_per_beat = (resolution_denom as usize / 4).max(1);

    // Capo resolution precedence: `--capo` on the command line wins, otherwise
    // fall back to whatever the file embedded in `\subtitle`. This means a
    // recording made with a capo round-trips cleanly without the user having
    // to remember and re-pass the same `--capo` value on playback — but they
    // can still override (e.g. take the capo off, or shift its position).
    let (effective_capo, capo_origin) = if let Some(spec) = capo_spec.as_deref() {
        let base = tab
            .tuning()
            .ok_or_else(|| anyhow!("'\\tuning' header is missing or unparseable"))?;
        let c = Capo::parse(spec, base.strings.len()).map_err(|e| anyhow!("{e}"))?;
        (Some(c), "flag")
    } else if let Some(c) = tab.capo() {
        (Some(c), "file")
    } else {
        (None, "")
    };

    // Effective tuning for wait-mode pitch comparison: tab's tuning + the
    // resolved capo. Frets in the tab are interpreted relative to this capo —
    // fret 0 means "the open string above the capo."
    let tuning_for_wait: Option<Tuning> = if wait {
        let base = tab
            .tuning()
            .ok_or_else(|| anyhow!("'\\tuning' header is missing or unparseable"))?;
        let c = effective_capo
            .clone()
            .unwrap_or_else(|| Capo::none(base.strings.len()));
        Some(c.apply(&base).map_err(|e| anyhow!("{e}"))?)
    } else {
        None
    };
    let header_capo = effective_capo;

    let mut output = if metronome {
        Some(OutputStream::open()?)
    } else {
        None
    };
    let click = output.as_ref().map(|o| metronome_click(o.sample_rate));

    let (mut input_state, mut input_buf) = if wait {
        let s = InputStream::open()?;
        let sr = s.sample_rate;
        (
            Some((s, Tuner::new(TunerMode::Chromatic, sr))),
            vec![0.0_f32; READ_CHUNK],
        )
    } else {
        (None, Vec::new())
    };

    let name_width = tab.tuning_names.iter().map(|n| n.len()).max().unwrap_or(0);

    eprintln!("Playback:   {}", path.display());
    if let Some(title) = tab.title.as_deref() {
        eprintln!("Title:      {title}");
    }
    if let Some(subtitle) = tab.subtitle_display() {
        // Strips any `; capo=...` machine annotation so the header line stays
        // human-readable; the resolved capo gets its own "Capo:" line below.
        eprintln!("Subtitle:   {subtitle}");
    }
    if let Some(name) = tuning_override.as_deref() {
        eprintln!("Transposed: {name} ({})", tab.tuning_names.join(" "));
    } else {
        eprintln!("Tuning:     {} (from file)", tab.tuning_names.join(" "));
    }
    if let Some(c) = &header_capo
        && !c.is_none()
    {
        let suffix = if capo_origin == "file" {
            " (from file)"
        } else {
            ""
        };
        if let Some(n) = c.is_uniform() {
            eprintln!("Capo:       {n} (uniform){suffix}");
        } else {
            eprintln!("Capo:       [{}] (partial){suffix}", c.serialize());
        }
    }
    eprintln!("Tempo:      {bpm} BPM, 1/{resolution_denom} notes ({ms_per_col} ms/col)");
    eprintln!("Metronome:  {}", if metronome { "on" } else { "off" });
    eprintln!("Wait mode:  {}", if wait { "on" } else { "off" });
    eprintln!(
        "Loop:       {}",
        if !repeat {
            "off".to_string()
        } else if loop_start == 0 && loop_end == tab.columns.len() {
            "full file".to_string()
        } else {
            format!("columns {loop_start}-{}", loop_end - 1)
        }
    );
    eprintln!();
    if !dropped.is_empty() {
        eprintln!(
            "Skipped:    {} note{} couldn't fit on the target tuning within fret 0–{MAX_FRET}",
            dropped.len(),
            if dropped.len() == 1 { "" } else { "s" },
        );
        // Show up to 8 unique note names so the user has a sense of *which*
        // notes are missing without flooding the terminal on a tab where the
        // transposition is largely a mismatch. Sort + dedup keeps the order
        // stable and the output short.
        let mut unique: Vec<String> = dropped.iter().map(|d| d.note.clone()).collect();
        unique.sort();
        unique.dedup();
        const PREVIEW: usize = 8;
        let shown: Vec<&str> = unique.iter().take(PREVIEW).map(String::as_str).collect();
        let extra = unique.len().saturating_sub(PREVIEW);
        if extra > 0 {
            eprintln!("            {} (+{extra} more)", shown.join(", "));
        } else {
            eprintln!("            {}", shown.join(", "));
        }
    }
    eprintln!("─────────────────────────────────────────────────");
    eprintln!("  Controls: type 'q' + Enter to stop  (or Ctrl-C)");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let stdin_rx = twanga_tui::spawn_line_reader();
    // One row per string + one for the position/progress line.
    let mut display = MultiLineDisplay::new(tab.tuning_names.len() + 1);

    'session: loop {
        for col_idx in loop_start..loop_end {
            if twanga_tui::is_shutdown_requested() {
                eprintln!();
                eprintln!("Playback stopped.");
                return Ok(());
            }
            if let Ok(input) = stdin_rx.try_recv() {
                if is_quit_input(&input) {
                    eprintln!();
                    eprintln!("Playback stopped.");
                    return Ok(());
                }
            }

            let column = &tab.columns[col_idx];
            // Duration progress within the current loop iteration. For
            // non-loop playback `loop_start = 0` and `loop_end =
            // tab.columns.len()`, so this naturally reads as the full-tab
            // elapsed/total. For section loops it resets at the top of
            // each iteration, which matches how the user thinks about
            // section practice.
            let elapsed_secs = (col_idx - loop_start) as u64 * ms_per_col as u64 / 1000;
            let total_secs = (loop_end - loop_start) as u64 * ms_per_col as u64 / 1000;
            let rows = render_playback_rows(
                &tab,
                col_idx,
                PLAYBACK_WINDOW_COLS,
                name_width,
                elapsed_secs,
                total_secs,
            );
            display.render(&rows)?;

            // Metronome tick on the downbeat of each beat.
            if col_idx % cols_per_beat == 0 {
                if let (Some(out), Some(click)) = (output.as_mut(), click.as_ref()) {
                    out.write(click);
                }
            }

            // Wait mode: pause for hits until the user plays a matching note.
            // Rests still consume one column of time so the metronome stays musical.
            if wait && !column.hits.is_empty() {
                let tuning = tuning_for_wait.as_ref().expect("wait mode requires tuning");
                wait_for_expected_note(
                    &mut input_state,
                    &mut input_buf,
                    &column.hits,
                    tuning,
                    &stdin_rx,
                )?;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(ms_per_col as u64));
            }
        }
        if !repeat {
            break 'session;
        }
    }

    eprintln!();
    eprintln!("Playback finished.");
    Ok(())
}

/// Parse the `--loop` spec into `(start, end, repeat)`. `start..end` is the
/// half-open column range to play; `repeat` is `false` for one-shot, `true`
/// to loop indefinitely.
fn parse_loop_spec(spec: Option<&str>, total: usize) -> Result<(usize, usize, bool)> {
    match spec {
        None => Ok((0, total, false)),
        Some("full") => Ok((0, total, true)),
        Some(s) => {
            let mut parts = s.split(':');
            let start_str = parts.next().unwrap_or("");
            let end_str = parts
                .next()
                .ok_or_else(|| anyhow!("--loop expects `START:END`, got '{s}'"))?;
            if parts.next().is_some() {
                return Err(anyhow!("--loop expects `START:END`, got '{s}'"));
            }
            let start: usize = start_str
                .parse()
                .map_err(|_| anyhow!("invalid loop start '{start_str}'"))?;
            let end: usize = end_str
                .parse()
                .map_err(|_| anyhow!("invalid loop end '{end_str}'"))?;
            if start >= end {
                return Err(anyhow!(
                    "loop start ({start}) must be less than end ({end})"
                ));
            }
            if end > total {
                return Err(anyhow!("loop end ({end}) exceeds column count ({total})"));
            }
            Ok((start, end, true))
        }
    }
}

/// Render the playback view: one row per string showing a window of columns
/// centred on `current_col`, with the current column bracketed. Last row is a
/// `[col / total]` progress + `M:SS / M:SS` elapsed line.
fn render_playback_rows(
    tab: &ParsedTab,
    current_col: usize,
    window_cols: usize,
    name_width: usize,
    elapsed_secs: u64,
    total_secs: u64,
) -> Vec<String> {
    let half = window_cols / 2;
    let start = current_col.saturating_sub(half);
    let end = (start + window_cols).min(tab.columns.len());

    let mut rows = Vec::with_capacity(tab.tuning_names.len() + 1);
    for (string_idx, name) in tab.tuning_names.iter().enumerate() {
        let mut content = String::new();
        for col_idx in start..end {
            let c = char_for_column(&tab.columns[col_idx], string_idx);
            if col_idx == current_col {
                content.push('[');
                content.push(c);
                content.push(']');
            } else {
                content.push(c);
            }
        }
        let padded = format!("{:<width$}", name, width = name_width);
        rows.push(format!("{padded} | {content}"));
    }

    let pad = format!("{:<width$}", "", width = name_width);
    rows.push(format!(
        "{pad}   col {}/{}  (bar {}, beat {})  {} / {}",
        current_col + 1,
        tab.columns.len(),
        current_col / tab.columns[0].duration_denom as usize + 1,
        (current_col % tab.columns[0].duration_denom as usize)
            / ((tab.columns[0].duration_denom / 4) as usize).max(1)
            + 1,
        format_mmss(elapsed_secs),
        format_mmss(total_secs),
    ));

    rows
}

/// Format seconds as `M:SS`. Shared between record + play; record uses it
/// alone, play uses it on both sides of a `/`.
fn format_mmss(total_secs: u64) -> String {
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("{m}:{s:02}")
}

fn char_for_column(col: &alphatex::TabColumn, string_idx: usize) -> char {
    let string_num = (string_idx + 1) as u8;
    for (s, fret) in &col.hits {
        if *s == string_num {
            return match *fret {
                n if n <= 9 => char::from_digit(n as u32, 10).unwrap(),
                _ => '+',
            };
        }
    }
    '-'
}

/// Generate a short metronome click — 50 ms of 1 kHz sine with a fast
/// exponential decay envelope, scaled down so it isn't piercing.
fn metronome_click(sample_rate: u32) -> Vec<f32> {
    let n = (sample_rate as f32 * 0.05) as usize;
    let mut buf = sine(Frequency(1000.0), sample_rate, n);
    exp_decay(&mut buf, sample_rate, 0.012);
    for s in buf.iter_mut() {
        *s *= 0.3;
    }
    buf
}

/// In wait mode, block until the user plays a frequency that matches one of
/// the expected `(string, fret)` hits within [`WAIT_MATCH_CENTS`] of the
/// target. Polls Ctrl-C / `q` so the user can still abort.
fn wait_for_expected_note(
    input_state: &mut Option<(InputStream, Tuner)>,
    buf: &mut [f32],
    expected: &[(u8, u8)],
    tuning: &Tuning,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) -> Result<()> {
    let (stream, tuner) = input_state.as_mut().expect("wait mode needs input");
    loop {
        if twanga_tui::is_shutdown_requested() {
            return Ok(());
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                return Ok(());
            }
        }
        let n = stream.read(buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            for r in tuner.take_readings() {
                if matches_any_expected(r.detected, expected, tuning) {
                    return Ok(());
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn matches_any_expected(detected: Frequency, expected: &[(u8, u8)], tuning: &Tuning) -> bool {
    for (string_num, fret) in expected {
        let string_idx = (*string_num as usize).saturating_sub(1);
        let Some(s) = tuning.strings.get(string_idx) else {
            continue;
        };
        let open_hz = s.open.to_frequency().hz();
        let target_hz = open_hz * 2_f32.powf(*fret as f32 / 12.0);
        let cents = 1200.0 * (detected.hz() / target_hz).log2();
        if cents.abs() < WAIT_MATCH_CENTS {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod slug_tests {
    use super::*;

    #[test]
    fn slugify_normalises_spaces_and_parens() {
        assert_eq!(slugify("Tenor Banjo (CGDA)"), "tenor-banjo-cgda");
    }

    #[test]
    fn slugify_handles_trailing_punctuation_without_dangling_hyphen() {
        assert_eq!(slugify("Drop D Guitar!"), "drop-d-guitar");
    }

    #[test]
    fn slugify_strips_leading_punctuation_too() {
        assert_eq!(slugify("...Standard Guitar"), "standard-guitar");
    }

    #[test]
    fn slugify_empty_input_falls_back_to_placeholder() {
        assert_eq!(slugify(""), "custom-tuning");
        assert_eq!(slugify("???"), "custom-tuning");
    }

    #[test]
    fn validate_slug_accepts_canonical_form() {
        assert!(validate_slug("standard-banjo").is_ok());
        assert!(validate_slug("drop-d-guitar").is_ok());
        assert!(validate_slug("a1").is_ok());
    }

    #[test]
    fn validate_slug_rejects_uppercase_and_underscores_and_edges() {
        assert!(validate_slug("Standard-Banjo").is_err());
        assert!(validate_slug("standard_banjo").is_err());
        assert!(validate_slug("-leading").is_err());
        assert!(validate_slug("trailing-").is_err());
        assert!(validate_slug("").is_err());
    }
}
