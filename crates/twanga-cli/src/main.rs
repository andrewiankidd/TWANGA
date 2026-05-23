mod audio_source;
mod bundled;
mod calibration;
mod import;
mod play_resume;
mod tunings;
mod wav;

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
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
    /// Subcommand is optional so `twanga` with no args reaches `main`
    /// (rather than clap exiting with "missing subcommand" before the
    /// banner can print). The None case is handled in `main` by
    /// printing the banner + clap's standard long-help.
    #[command(subcommand)]
    command: Option<Command>,
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
        /// Silence-gate threshold (window RMS, linear amplitude). Detection is
        /// skipped when the 8192-sample window average is below this value.
        /// Default 0.005 (≈ -46 dB) catches a quiet room while staying below
        /// any plucked note. Lower to catch quieter plucks (more cable-hum /
        /// noise false positives); higher to reject more noise. Range 0..1.
        /// The GUI exposes the same setting as a slider over the level meter.
        #[arg(long, value_name = "RMS")]
        silence_rms: Option<f32>,
    },
    /// Play back a `.alphatex` recording. Scrolling cursor view, optional
    /// metronome click on each beat, optional "wait" mode that pauses until
    /// you play the expected note.
    Play {
        /// Path to a `.alphatex` file. Omit to open an interactive
        /// picker that scans bundled examples, bundled patterns, and
        /// `./recordings/` for `.alphatex` files — same library the
        /// GUI's Playback screen shows.
        path: Option<PathBuf>,
        /// Re-tune the tab to a different instrument's tuning. Notes are
        /// transposed by absolute pitch — e.g. play a uke tab on banjo with
        /// `--tuning standard-banjo`. Notes outside the target instrument's
        /// playable range are silently dropped (use `--transpose-mode
        /// octave-shift` to retry them at ±octaves first). Omit or pass
        /// `--tuning` with no value to be prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        tuning: Option<String>,
        /// How to handle notes that don't fit on the target tuning's
        /// fretboard during `--tuning` transposition. `drop` (default)
        /// silently omits them and reports a "Skipped:" pre-flight
        /// summary; `octave-shift` retries each unreachable note at
        /// progressively wider ±12-semitone offsets before giving up.
        /// `octave-shift` is the standard cross-instrument convention
        /// (TuxGuitar / MuseScore behaviour) — preserves melodic
        /// contour at the cost of register. Particularly relevant for
        /// banjo→ukulele where bass drone notes would otherwise vanish.
        #[arg(long, value_parser = ["drop", "octave-shift"], default_value = "drop")]
        transpose_mode: String,
        /// Override the tempo from the file (BPM). Omit or pass `--bpm` with
        /// no value to keep the file's tempo (no prompt — there's already a
        /// sensible default from the file).
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        bpm: Option<String>,
        /// Disable the metronome click (default: on).
        #[arg(long)]
        no_metronome: bool,
        /// Wait for the user to play each note before advancing the cursor.
        /// Equivalent to `--policy wait`. Mutually exclusive with
        /// `--policy <other>` — pick one verb for what the playhead does.
        #[arg(long, conflicts_with = "policy")]
        wait: bool,
        /// Playhead behaviour for the session. `wait` pauses on each note
        /// until you play it (slow drill, same as `--wait`); `tight` and
        /// `casual` run at tempo and score each note by proximity to the
        /// expected onset (±50 ms / ±150 ms respectively); `free` scrolls
        /// at tempo with no verification. Defaults to `free` if neither
        /// `--wait` nor `--policy` is passed (matches the pre-Ship-2
        /// behaviour). Score-mode prints a hit/late/missed/wrong-pitch
        /// summary at the end of the run.
        #[arg(long, value_parser = ["wait", "tight", "casual", "free"])]
        policy: Option<String>,
        /// Replay a mono PCM WAV file in place of the live mic. The
        /// file is paced to wall-clock at its own sample rate so the
        /// playback loop ticks against it identically to a real
        /// input stream. Used for deterministic end-to-end testing
        /// (integration suite under `tests/play_from_file.rs`) and
        /// for repeatable demos on a headless box. No-op when the
        /// policy doesn't need audio (`--policy free`).
        #[arg(long, value_name = "PATH")]
        from_file: Option<PathBuf>,
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
        /// Pre-roll / count-in ticks before playback starts. Always audible
        /// (independent of `--no-metronome` — the count needs to be heard
        /// even when the main run is silent). Default 4 (one bar at 4/4).
        /// Range 0–16; 0 disables. Omit or pass `--pre-roll` bare to be
        /// prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        pre_roll: Option<String>,
        /// Skip the "Resume at col N?" prompt by automatically accepting
        /// the saved bookmark for this file (if one exists). No-op when
        /// the file has no bookmark — playback still starts from
        /// column 0.
        #[arg(long, conflicts_with = "no_resume")]
        resume: bool,
        /// Skip the resume prompt by automatically *declining* the
        /// saved bookmark. Useful in scripts where you want a clean
        /// start regardless of history.
        #[arg(long, conflicts_with = "resume")]
        no_resume: bool,
        /// Silence-gate threshold for wait-mode pitch detection (window RMS,
        /// linear amplitude). Default 0.005 (≈ -46 dB). Same semantics as
        /// `twanga tune --silence-rms` — no-op when `--wait` isn't passed.
        #[arg(long, value_name = "RMS")]
        silence_rms: Option<f32>,
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
        /// Pre-roll / count-in ticks before recording starts. Always audible
        /// (independent of `--no-metronome`). Default 4 (one bar at 4/4).
        /// Range 0–16; 0 disables. Omit or pass `--pre-roll` bare to be
        /// prompted.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        pre_roll: Option<String>,
        /// Human-readable title for the recording — written to `\title` in the
        /// alphaTex header AND used to derive the filename
        /// (`<slug>-<unix-secs>.alphatex` if provided, `recording-<unix-secs>`
        /// otherwise). Omit or pass `--title` with no value to be prompted.
        /// Accept the blank default to keep the pre-title filename shape.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        title: Option<String>,
        /// Silence-gate threshold (window RMS, linear amplitude). Default 0.005
        /// (≈ -46 dB). Same semantics as `twanga tune --silence-rms`.
        #[arg(long, value_name = "RMS")]
        silence_rms: Option<f32>,
    },
    /// List available audio input devices.
    Devices,
    /// Import a tab file into the user library. Accepts alphaTex
    /// (`.alphatex`), MusicXML (`.musicxml` / `.xml` / `.mxl`), and
    /// Standard MIDI File (`.mid` / `.midi`) inputs. Non-alphaTex
    /// sources are converted on the way in via the canonical
    /// `AlphaTexWriter`, so the saved file is bit-for-bit identical
    /// to what `twanga record` would have produced from the same
    /// notes. Lands at `<data-root>/library/`; the picker on
    /// `twanga play` and the GUI Playback library both surface it.
    Import {
        /// Path to the source file. Format is detected from the
        /// extension unless `--from` is set.
        input: PathBuf,
        /// Force the source format. Accepts `alphatex`, `musicxml`,
        /// `mxl`, or `midi`. Use when the extension is missing or
        /// wrong (a `.txt` containing alphaTex, for example).
        #[arg(long, value_parser = ["alphatex", "musicxml", "mxl", "midi", "mid", "abc", "ascii", "ascii-tab", "tab"])]
        from: Option<String>,
        /// Override the title. Otherwise we take the source's
        /// embedded title (`\title` for alphaTex, `<work-title>` for
        /// MusicXML) or fall back to `imported`. The slug derived
        /// from this title is part of the destination filename.
        #[arg(long)]
        title: Option<String>,
    },
    /// Convert a tab file from one format to another (stateless —
    /// no library involvement, both paths are explicit). Useful for
    /// scripting bulk MusicXML-to-alphaTex transforms before importing
    /// or for one-off sharing of a converted file. The output is
    /// always alphaTex today; other targets land if a use case
    /// shows up.
    Convert {
        /// Path to the source file. Format detected from extension
        /// unless `--from` is set.
        input: PathBuf,
        /// Destination path. Will be overwritten if it already
        /// exists.
        #[arg(long)]
        out: PathBuf,
        /// Force the source format. Accepts `alphatex`, `musicxml`,
        /// `mxl`, or `midi`.
        #[arg(long, value_parser = ["alphatex", "musicxml", "mxl", "midi", "mid", "abc", "ascii", "ascii-tab", "tab"])]
        from: Option<String>,
    },
    /// Manage user-defined tunings stored at the platform config dir alongside
    /// the built-in presets.
    Tunings {
        #[command(subcommand)]
        action: TuningsAction,
    },
    /// Browse + play bundled rhythm + picking drills. Read-only —
    /// patterns are shipped at `assets/patterns/` with the binary
    /// (no add / remove subcommands, unlike `tunings`). Use the
    /// underlying `.alphatex` files with `twanga play <path>` if you
    /// want to feed them through any of `play`'s flags.
    Patterns {
        #[command(subcommand)]
        action: Option<PatternsAction>,
    },
    /// Edit an `.alphatex` file in place — cell-level fret changes,
    /// column insert / delete / clear, title + BPM. Scriptable
    /// (non-interactive) counterpart to the GUI Editor screen. Each
    /// `edit` invocation does one operation and writes back; chain
    /// them in a shell script for batch edits.
    Edit {
        /// Path to the `.alphatex` file to mutate.
        path: PathBuf,
        /// Write the modified tab to a different file instead of
        /// overwriting `path`. Useful for branching edits without
        /// touching the original.
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(subcommand)]
        action: EditAction,
    },
    /// Print the per-feature documentation embedded in the binary. With no
    /// argument, lists the available pages. Markdown is printed raw to
    /// stdout; pipe through `glow`, `mdcat`, or `bat -l md` for rendering.
    Docs {
        /// Feature slug — one of `tuner`, `recorder`, `playback`,
        /// `patterns`, `editor`, `tunings`. Omit to list the available
        /// pages.
        feature: Option<String>,
    },
    /// Measure your audio chain's output→input round-trip latency,
    /// or set it manually for setups (headphones, line-in, no-mic)
    /// where the acoustic loop isn't available. By default,
    /// `twanga calibrate` runs an interactive wizard that picks the
    /// right method based on a couple of setup questions; pass one
    /// of the flags below to skip the wizard for scripting. Result
    /// is persisted under the data root and consumed by
    /// `twanga play`'s proximity-score modes (tight / casual) —
    /// without calibration those modes systematically score on-time
    /// plucks as Late.
    Calibrate {
        /// Print the currently-stored calibration without measuring.
        /// Useful in scripts that just want to read the value back.
        #[arg(long, conflicts_with_all = ["round_trip", "manual"])]
        show: bool,
        /// Skip the wizard and run the round-trip measurement
        /// (plays clicks through your speakers, captures via mic).
        /// Requires both speakers + mic that can acoustically loop;
        /// errors out cleanly if no clicks are detected.
        #[arg(long, conflicts_with = "manual")]
        round_trip: bool,
        /// Skip the wizard and save a manually-entered value in
        /// milliseconds. Use when you know your input pipeline
        /// latency from your interface's spec sheet, or when the
        /// acoustic round-trip isn't possible (headphones / line-in).
        /// Range 0..=1000.
        #[arg(long, value_name = "MS")]
        manual: Option<u32>,
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
    /// Remove a user-defined tuning by slug. Built-in tunings can't be
    /// removed (they're compiled into the binary). Omit `--slug` to be
    /// prompted with a menu of user tunings.
    Remove {
        /// Slug of the user tuning to remove. Omit or pass `--slug` with no
        /// value to pick interactively from the user tunings on disk.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        slug: Option<String>,
        /// Skip the "delete '<slug>'? (y/N)" confirmation prompt. Useful
        /// for scripts; interactive users should leave this off.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum EditAction {
    /// Set a single cell. `string` is 1-based (string 1 = the highest
    /// pitch / top of the tab); `column` is 0-based. `fret` is any
    /// non-negative integer (no upper cap — extended-range
    /// instruments use whatever they need).
    Set { column: usize, string: u8, fret: u8 },
    /// Clear a single cell. Same indexing as `set`.
    Clear { column: usize, string: u8 },
    /// Clear every cell in the given column (rest the entire beat).
    ClearCol { column: usize },
    /// Insert a blank column. `--after` is 0-based; omit to append at
    /// the end. Inserting after `N-1` produces a blank column at
    /// position `N` (i.e. one past where you specified).
    InsertCol {
        #[arg(long)]
        after: Option<usize>,
    },
    /// Delete the column at `column` (0-based). The columns after it
    /// shift down by one.
    DeleteCol { column: usize },
    /// Set the `\title` directive.
    Title { text: String },
    /// Set the `\tempo` directive (BPM, 20–400).
    Bpm { bpm: u32 },
}

#[derive(Subcommand)]
enum PatternsAction {
    /// List all bundled patterns, grouped by tradition with difficulty
    /// markers. Read-only catalog view — no patterns are user-defined.
    List,
    /// Print the path to the bundled patterns manifest.
    Path,
    /// Play a specific pattern by its manifest id. Scriptable
    /// equivalent of picking from the interactive `twanga patterns`
    /// menu. The chosen pattern plays with `--loop full` defaulted
    /// (override with `--no-loop` to play through once).
    Play {
        /// Manifest id of the pattern to play (e.g. `bum-diddy-simple`).
        /// Run `twanga patterns list` to see the available ids.
        id: String,
        /// Disable the metronome click (default: on).
        #[arg(long)]
        no_metronome: bool,
        /// Wait for the user to play each note before advancing.
        #[arg(long)]
        wait: bool,
        /// Skip the default `--loop full` and play through once.
        #[arg(long)]
        no_loop: bool,
        /// Override the tempo from the pattern's `\tempo` line.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        bpm: Option<String>,
    },
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
            // User-defined tunings don't expose fret_offset through the prompt
            // flow yet — covers 99% of instruments (everything fretted from
            // nut to body has offset 0). The banjo 5-string drone is the only
            // case so far that needs non-zero, and that lives in the bundled
            // `presets.toml`. If users start defining 5-string banjo variants
            // by hand, they can edit `$CONFIG/twanga/tunings.toml` to set it.
            fret_offset: 0,
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

/// `twanga tunings remove [--slug <slug>] [--force]` — closes the
/// reverse-parity gap with the GUI's delete button on user-tunings
/// rows. Built-in tunings are compiled into the binary and can't be
/// "removed"; the helper rejects those upfront.
fn run_tunings_remove(slug_arg: Option<String>, force: bool) -> Result<()> {
    let user_entries = tunings::load_user_tunings()?;
    if user_entries.is_empty() {
        eprintln!("No user-defined tunings to remove.");
        eprintln!("(Run `twanga tunings add` to create one, or `twanga tunings list`");
        eprintln!("to see what's available — built-in tunings can't be removed.)");
        return Ok(());
    }

    // Resolve the target slug: explicit `--slug X` wins; otherwise prompt
    // with a menu of user tunings. The three-form flag pattern means
    // omitting `--slug` entirely also lands here (interactive default).
    let slug = match flag_value(&slug_arg) {
        Some(s) => s.to_string(),
        None => {
            let labels: Vec<String> = user_entries
                .iter()
                .map(|e| format!("{} — {}", e.slug, e.name))
                .collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let idx = twanga_tui::select("Which user tuning to remove?", &label_refs)?;
            user_entries[idx].slug.clone()
        }
    };

    // Pre-flight validation so the user sees a clear error before the
    // confirmation prompt (rather than the prompt + then a failure).
    if Tuning::builtin_slugs().contains(&slug.as_str()) {
        return Err(anyhow!(
            "'{slug}' is a built-in preset — built-ins can't be removed \
             (they're compiled into the binary). Pick a user-defined slug \
             from `twanga tunings list`."
        ));
    }
    let target = user_entries
        .iter()
        .find(|e| e.slug == slug)
        .ok_or_else(|| {
            let available: Vec<String> = user_entries.iter().map(|e| e.slug.clone()).collect();
            anyhow!(
                "no user tuning with slug '{slug}'. Available: {}",
                available.join(", ")
            )
        })?;

    if !force {
        eprintln!("About to remove user tuning:");
        eprintln!("  slug: {}", target.slug);
        eprintln!("  name: {}", target.name);
        let pitches: Vec<String> = target.strings.iter().map(|s| s.name.clone()).collect();
        eprintln!("  strings: {}", pitches.join(" "));
        let confirmed: bool = twanga_tui::prompt_parsed("Delete? (y/N)", false, |s| {
            match s.trim().to_lowercase().as_str() {
                "" | "n" | "no" => Ok(false),
                "y" | "yes" => Ok(true),
                other => Err(format!("expected y/yes or n/no, got '{other}'")),
            }
        })?;
        if !confirmed {
            eprintln!("Cancelled — '{}' kept.", target.slug);
            return Ok(());
        }
    }

    let path = tunings::remove_user_tuning(&slug)?;
    eprintln!("Removed '{slug}' from {}.", path.display());
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
/// `"Tenor Banjo (CGDA)"` → `"tenor-banjo-cgda"`. Used by the recorder
/// (filename derivation) and by the importer's library-write helper —
/// both want the same kebab-case behaviour, so `pub(crate)` exposes a
/// single implementation across the crate's modules.
pub(crate) fn slugify(name: &str) -> String {
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

const DEFAULT_PRE_ROLL: u32 = 4;

/// Resolve `--pre-roll <N>` for both `record` and `play`. Three-form
/// pattern: explicit value passes through, bare flag prompts, omission
/// also prompts. Default 4 ticks (one bar at 4/4). Range 0–16.
fn resolve_pre_roll(arg: Option<String>) -> Result<u32> {
    if let Some(s) = flag_value(&arg) {
        let n: u32 = s
            .parse()
            .map_err(|_| anyhow!("invalid pre-roll '{s}' (expected an integer)"))?;
        validate_pre_roll(n)?;
        return Ok(n);
    }
    twanga_tui::prompt_parsed("Pre-roll ticks (0 to disable)", DEFAULT_PRE_ROLL, |s| {
        let n: u32 = s
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        validate_pre_roll(n).map_err(|e| e.to_string())?;
        Ok(n)
    })
}

fn validate_pre_roll(n: u32) -> Result<()> {
    if n <= 16 {
        Ok(())
    } else {
        Err(anyhow!("pre-roll out of range (0–16): {n}"))
    }
}

/// Resolve the resume-bookmark decision for `twanga play`. Looks up
/// any saved bookmark for `file_path`; returns:
///   - `Ok(Some(col))` if the user (or `--resume`) opted to jump to
///     a saved column.
///   - `Ok(None)` if there's no bookmark, `--no-resume` was passed,
///     the user declined the prompt, or stdin isn't a TTY (default
///     for scripts: don't resume).
///
/// Mirrors the GUI's banner-then-resume flow but on a per-file
/// basis since the CLI usually loads with an explicit path.
fn resolve_resume_choice(
    file_path: &Path,
    parsed: &ParsedTab,
    resume_flag: bool,
    no_resume_flag: bool,
) -> Result<Option<u64>> {
    if no_resume_flag {
        return Ok(None);
    }
    let Some(bm) = play_resume::lookup(file_path).unwrap_or(None) else {
        return Ok(None);
    };
    let total_cols = parsed.columns.len() as u64;
    // Defensive: a saved column past the file's end (e.g. file was
    // edited shorter) is meaningless. Clear the stale bookmark and
    // start from the top.
    if bm.column >= total_cols {
        let _ = clear_resume_bookmark(file_path);
        return Ok(None);
    }
    if resume_flag {
        eprintln!(
            "Resuming at column {} / {} (saved {})",
            bm.column + 1,
            total_cols,
            humanise_resume_age(bm.when),
        );
        return Ok(Some(bm.column));
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        // Non-interactive default: don't auto-resume — scripts get a
        // predictable start. Use `--resume` to opt in.
        return Ok(None);
    }
    let label = bm.title.clone().unwrap_or_else(|| {
        file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("this tab")
            .to_string()
    });
    let prompt = format!(
        "Resume \"{}\" at column {} / {} (saved {})? (Y/n)",
        label,
        bm.column + 1,
        total_cols,
        humanise_resume_age(bm.when),
    );
    let accept: bool =
        twanga_tui::prompt_parsed(&prompt, true, |s| match s.trim().to_lowercase().as_str() {
            "" | "y" | "yes" => Ok(true),
            "n" | "no" => Ok(false),
            other => Err(format!("expected y/yes or n/no, got '{other}'")),
        })?;
    Ok(if accept { Some(bm.column) } else { None })
}

/// Best-effort "5 minutes ago" / "2 hours ago" / "yesterday" /
/// "2026-05-21" rendering for the resume prompt. Keeps the prompt
/// readable without dragging in a chrono dep.
fn humanise_resume_age(when_unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if when_unix == 0 || when_unix > now {
        return "just now".to_string();
    }
    let secs = now - when_unix;
    if secs < 60 {
        return "just now".to_string();
    }
    if secs < 60 * 60 {
        let m = secs / 60;
        return format!("{m} minute{} ago", if m == 1 { "" } else { "s" });
    }
    if secs < 60 * 60 * 24 {
        let h = secs / (60 * 60);
        return format!("{h} hour{} ago", if h == 1 { "" } else { "s" });
    }
    if secs < 60 * 60 * 24 * 7 {
        let d = secs / (60 * 60 * 24);
        return if d == 1 {
            "yesterday".to_string()
        } else {
            format!("{d} days ago")
        };
    }
    let weeks = secs / (60 * 60 * 24 * 7);
    format!("{weeks} week{} ago", if weeks == 1 { "" } else { "s" })
}

/// Thin wrapper to swallow the result of clearing a stale bookmark.
/// Best-effort — if the user has no config dir we don't care.
fn clear_resume_bookmark(file_path: &Path) -> Result<()> {
    let Some(bp) = play_resume::bookmarks_file_path() else {
        return Ok(());
    };
    play_resume::clear_at(&bp, file_path)
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

/// Toggle for `--pause` / "p + Enter" on record + play. Lower-case
/// `p` is the documented control; `pause` is a typo-friendly long
/// form following the same pattern as `q` / `quit`.
fn is_pause_input(input: &str) -> bool {
    matches!(input, "p" | "pause")
}

/// Undo-last-column for `record` while paused. `u + Enter` parity with
/// the GUI Recorder's "Undo last column" button. Long form `undo`
/// matches the `quit` / `pause` aliasing pattern.
fn is_undo_input(input: &str) -> bool {
    matches!(input, "u" | "undo")
}

/// Silence-threshold step-down keypress. `[ + Enter` halves the gate
/// (≈ -6 dB), making detection trigger on quieter input. CLI parity
/// for the GUI's mic-meter slider.
fn is_threshold_down(input: &str) -> bool {
    matches!(input, "[")
}

/// Silence-threshold step-up keypress. `] + Enter` doubles the gate
/// (≈ +6 dB), making detection require louder input. CLI parity
/// for the GUI's mic-meter slider.
fn is_threshold_up(input: &str) -> bool {
    matches!(input, "]")
}

/// Window length for the per-session auto noise-floor calibration.
/// 3s gives ~30 RMS samples at 100 ms each — plenty for a robust
/// 10th-percentile floor measurement, while short enough that the
/// startup pause isn't annoying.
const NOISE_CALIBRATION_SECONDS: f32 = 3.0;

/// Display state for the auto noise-floor calibration during a CLI
/// session. Tracks whether we've already printed the "calibration
/// complete" line for the current run so it only scrolls past once.
struct CliCalibration {
    announced: bool,
}

impl CliCalibration {
    fn new() -> Self {
        Self { announced: false }
    }

    /// Status-line text while a calibration is in progress. `None` when
    /// no calibration is active — the caller should fall back to the
    /// usual status (reading display, "(play any note)", etc).
    fn status(&self, tuner: &Tuner) -> Option<String> {
        let p = tuner.calibration_progress()?;
        let remaining =
            p.samples_total.saturating_sub(p.samples_collected) as f32 / tuner.sample_rate() as f32;
        Some(format!("Calibrating noise floor… {remaining:.1}s"))
    }

    /// Once-per-session announcement printed after a calibration
    /// completes. Subsequent calls return `None` until a fresh
    /// calibration is started. Caller emits this as its own line
    /// (it doesn't replace the status line) — the next live-reading
    /// tick redraws below it.
    fn take_announcement(&mut self, tuner: &Tuner) -> Option<String> {
        if self.announced || tuner.calibration_progress().is_some() {
            return None;
        }
        let r = tuner.last_calibration()?;
        self.announced = true;
        Some(format!(
            "Noise floor: {:.1} dB → gate at {:.1} dB ({:.5} RMS)",
            20.0 * r.floor_rms.log10(),
            20.0 * r.threshold_rms.log10(),
            r.threshold_rms,
        ))
    }
}

/// Step the tuner's silence threshold by ±6 dB (×2 / ×0.5 in linear
/// amplitude). 6 dB steps because the useful threshold range spans
/// roughly 0.001..0.05, which is six doublings — about seven
/// keypresses gets across the entire usable band. Prints the new
/// value (in both linear RMS and dB) on its own line so the user
/// sees what they've set; the next tick of the status-line refresh
/// redraws the live reading underneath.
fn step_silence_threshold(tuner: &mut Tuner, up: bool) {
    let current = tuner.silence_rms();
    let factor = if up { 2.0_f32 } else { 0.5_f32 };
    // Clamp to a sane range. 0.00001 is well below any plausible
    // signal; 0.5 is louder than any plucked-string note ever gets.
    let next = (current * factor).clamp(0.000_01, 0.5);
    tuner.set_silence_rms(next);
    let db = 20.0 * next.log10();
    eprintln!("\n[silence: {next:.5} RMS ({db:.1} dB)]");
}

fn run_chromatic(mut tuner: Tuner, mut stream: InputStream) -> Result<()> {
    let mut status = StatusLine::new();
    let use_color = status.is_terminal();
    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();
    let mut cal = CliCalibration::new();

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
            if is_threshold_down(&input) {
                step_silence_threshold(&mut tuner, false);
            } else if is_threshold_up(&input) {
                step_silence_threshold(&mut tuner, true);
            }
        }
        let n = stream.read(&mut buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            if let Some(text) = cal.take_announcement(&tuner) {
                eprintln!("\n{text}");
            }
            if let Some(text) = cal.status(&tuner) {
                // During calibration, the status line shows the countdown.
                // Readings still accumulate (pitch detection runs in
                // parallel) but we drain + discard them — they'd flash by
                // too fast to read while the countdown is also updating.
                status.update(&text)?;
                tuner.take_readings();
            } else {
                for r in tuner.take_readings() {
                    status.update(&format_chromatic_row(&r, use_color))?;
                }
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
    let mut cal = CliCalibration::new();
    let mut last_cal_status: Option<String> = None;

    loop {
        if twanga_tui::is_shutdown_requested() {
            return Ok(());
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                return Ok(());
            }
            if is_threshold_down(&input) {
                step_silence_threshold(&mut tuner, false);
            } else if is_threshold_up(&input) {
                step_silence_threshold(&mut tuner, true);
            }
        }
        let n = stream.read(&mut buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            if let Some(text) = cal.take_announcement(&tuner) {
                eprintln!("\n{text}");
                last_cal_status = None;
            }
            // While calibrating, render the per-string display normally
            // but also print a transient countdown line above it the first
            // time we see each new "X.Xs" tick — gives the user a sense
            // of progress without re-rendering the multi-string block.
            if let Some(text) = cal.status(&tuner) {
                if last_cal_status.as_deref() != Some(&text) {
                    eprintln!("[{text}]");
                    last_cal_status = Some(text);
                }
                tuner.take_readings();
            } else {
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
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn run_tuner(mode: TunerMode, silence_rms: Option<f32>) -> Result<()> {
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
    eprintln!("  Controls: 'q' + Enter to stop  (or Ctrl-C)");
    eprintln!("            '[' / ']' + Enter to step silence threshold ∓ 6 dB");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let strings = match &mode {
        TunerMode::Chromatic => None,
        TunerMode::Strings(t) => Some(t.strings.clone()),
    };

    let mut tuner = Tuner::new(mode, sample_rate);
    // Explicit `--silence-rms <N>` skips auto-calibration entirely: the
    // user has stated a value and we respect it. Otherwise, measure the
    // noise floor over the next NOISE_CALIBRATION_SECONDS and set the
    // gate accordingly.
    if let Some(rms) = silence_rms {
        tuner.set_silence_rms(rms);
    } else {
        tuner.start_noise_calibration(NOISE_CALIBRATION_SECONDS);
    }
    match strings {
        None => run_chromatic(tuner, stream),
        Some(s) => run_strings(tuner, stream, s),
    }
}

/// Highest fret the recorder will accept on any string. Anything past this is
/// treated as garbage / out-of-range and silently dropped.
const MAX_FRET: u8 = 20;

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
    // Single source of truth for the recordings dir — same call the
    // picker (`bundled::scan_recordings`) uses, so writes + scans
    // agree across home / portable modes.
    let dir = bundled::recordings_dir_path();
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
    pre_roll: u32,
    title: Option<String>,
    silence_rms: Option<f32>,
) -> Result<()> {
    let mut stream = InputStream::open()?;
    let sample_rate = stream.sample_rate;
    let ms_per_col = 240_000 / (bpm * resolution_denom);
    // Same beat-boundary derivation `run_playback` uses: at 1/8 resolution,
    // every other column is a beat; at 1/16, every fourth; etc.
    let cols_per_beat = (resolution_denom as usize / 4).max(1);

    // Open an output stream if the user wants a metronome OR a pre-roll —
    // pre-roll is always audible, even when the main metronome is off.
    let mut output = if metronome || pre_roll > 0 {
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
    // Auto-calibrate the silence gate unless the user pinned it via flag.
    // Calibration overlaps the recording's first 3s; hits detected during
    // that window are discarded (see the loop below) so a noisy first
    // moment doesn't pollute the take.
    if let Some(rms) = silence_rms {
        tuner.set_silence_rms(rms);
    } else {
        tuner.start_noise_calibration(NOISE_CALIBRATION_SECONDS);
    }
    let mut cal = CliCalibration::new();

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
    eprintln!("Pre-roll:   {pre_roll}");
    eprintln!("Saving to:  {}", recording_path.display());
    eprintln!();
    eprintln!("─────────────────────────────────────────────────");
    eprintln!("  Controls: 'q' stop · 'p' pause/resume · 'u' undo last col (while paused)");
    eprintln!("            '[' / ']' step silence threshold ∓ 6 dB");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let n_rows = recorder.string_count();
    // +1 row for the duration / column-count status appended below the tab.
    // Each block recreates the display so the status sits at the bottom of
    // whatever block is currently growing.
    let mut display: Option<MultiLineDisplay> = None;
    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();

    if run_pre_roll(pre_roll, bpm, &mut output, click.as_ref(), &stdin_rx)? {
        return finalize_recording(
            &mut recording_writer,
            &recording_path,
            "Recording cancelled during pre-roll.",
        );
    }
    let mut total_samples: u64 = 0;
    let mut total_columns: u64 = 0;
    // Aggregate count of detected pitches that no string + fret combo could
    // reach on the active (post-capo) tuning. Per the parity audit's
    // "couldn't fit on fretboard" item — silent drops were the previous
    // behaviour. Per-frame logging is too noisy; aggregate is enough.
    let mut total_dropped: u64 = 0;
    let mut paused = false;

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
            if is_pause_input(&input) {
                paused = !paused;
                if paused {
                    eprintln!(
                        "\n[paused — 'p' + Enter to resume, 'u' + Enter to undo last column]"
                    );
                } else {
                    eprintln!("[resuming]");
                }
            } else if is_undo_input(&input) {
                // Only honour undo while paused, matching the GUI
                // recorder's Undo-last-column button (which is only
                // visible while the recorder is paused). Doing it
                // while actively recording would race the tick driver.
                if paused {
                    match recorder.undo_last_column() {
                        Some(_marks) => {
                            total_columns = total_columns.saturating_sub(1);
                            eprintln!("[undid last column — {total_columns} columns remaining]");
                        }
                        None => {
                            eprintln!("[nothing to undo]");
                        }
                    }
                } else {
                    eprintln!("[undo only works while paused — press 'p' + Enter first]");
                }
            } else if is_threshold_down(&input) {
                step_silence_threshold(&mut tuner, false);
            } else if is_threshold_up(&input) {
                step_silence_threshold(&mut tuner, true);
            }
        }
        let n = stream.read(&mut buf);
        if paused {
            // While paused: still drain the audio buffer to keep the device
            // from backing up, but don't feed the tuner or advance the
            // recorder. Time stops; resuming picks up at the same column.
            std::thread::sleep(std::time::Duration::from_millis(20));
            continue;
        }
        if n > 0 {
            total_samples += n as u64;
            tuner.feed(&buf[..n]);
            if let Some(text) = cal.take_announcement(&tuner) {
                eprintln!("\n{text}");
            }
            // Drop tuner readings entirely while calibration is running —
            // we don't want to commit hits to the recording before the
            // gate is properly set, and the user wouldn't see them with
            // pre-roll clicks playing anyway.
            if cal.status(&tuner).is_some() {
                tuner.take_readings();
            } else {
                for r in tuner.take_readings() {
                    match effective.match_to_fret(r.detected, MAX_FRET) {
                        Some(m) => recorder.record_hit(m.string_idx, m.fret),
                        None => total_dropped += 1,
                    }
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
    // Bare `twanga` with no subcommand: banner has already printed; follow
    // it with clap's standard long-help so the user sees the splash AND
    // the same usage text clap's missing-subcommand error would have
    // shown. Exit 0 because typing `twanga` to discover what's available
    // isn't an error.
    let Some(command) = cli.command else {
        let mut cmd = Cli::command();
        cmd.print_long_help()?;
        println!();
        return Ok(());
    };
    match command {
        Command::Tune {
            tuning,
            capo,
            silence_rms,
        } => {
            let mode = resolve_mode(tuning)?;
            let mode = match mode {
                TunerMode::Chromatic => TunerMode::Chromatic,
                TunerMode::Strings(t) => {
                    let c = resolve_capo(capo, &t)?;
                    let effective = c.apply(&t).map_err(|e| anyhow!("{e}"))?;
                    TunerMode::Strings(effective)
                }
            };
            run_tuner(mode, silence_rms)?;
        }
        Command::Record {
            tuning,
            bpm,
            resolution,
            block_width,
            capo,
            no_metronome,
            pre_roll,
            title,
            silence_rms,
        } => {
            let t = resolve_tuning(tuning)?;
            let c = resolve_capo(capo, &t)?;
            let bpm = resolve_bpm(bpm)?;
            let denom = resolve_resolution(resolution)?;
            let bw = resolve_block_width(block_width)?;
            let pre_roll = resolve_pre_roll(pre_roll)?;
            let title = resolve_title(title)?;
            run_recorder(
                t,
                c,
                bpm,
                denom,
                bw,
                !no_metronome,
                pre_roll,
                title,
                silence_rms,
            )?;
        }
        Command::Play {
            path,
            tuning,
            transpose_mode,
            bpm,
            no_metronome,
            wait,
            policy,
            from_file,
            loop_spec,
            capo,
            pre_roll,
            resume,
            no_resume,
            silence_rms,
        } => {
            // Resolve the file to play: an explicit path always wins;
            // otherwise drop into the interactive picker that merges
            // bundled examples + bundled patterns + the user's
            // `./recordings/` directory. Same library the GUI's
            // Playback screen shows.
            let path = match path {
                Some(p) => p,
                None => match prompt_play_target()? {
                    Some(p) => p,
                    None => {
                        eprintln!("Nothing selected — exiting.");
                        return Ok(());
                    }
                },
            };
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
            let pre_roll = resolve_pre_roll(pre_roll)?;
            let transpose_mode = match transpose_mode.as_str() {
                "octave-shift" => alphatex::TransposeMode::OctaveShift,
                _ => alphatex::TransposeMode::Drop,
            };
            // Resume bookmark resolution. Three states for the user
            // (`--resume`, `--no-resume`, neither) → either skip the
            // prompt one way or the other, or ask interactively.
            let resume_col = resolve_resume_choice(&path, &parsed, resume, no_resume)?;
            // Resolve `--wait` + `--policy` into a single PlaybackPolicy.
            // `--wait` is the legacy verb (kept working for back-compat);
            // `--policy <name>` is the Ship 2 surface. Clap rejects passing
            // both, so exactly one path runs. Default (neither passed) is
            // `FreePlay` — same behaviour the CLI shipped before Ship 2,
            // so existing scripts don't suddenly grow a scoring summary.
            let policy = if wait {
                twanga_tabs::playback::PlaybackPolicy::wait()
            } else {
                match policy.as_deref() {
                    Some("wait") => twanga_tabs::playback::PlaybackPolicy::wait(),
                    Some("tight") => twanga_tabs::playback::PlaybackPolicy::tight(),
                    Some("casual") => twanga_tabs::playback::PlaybackPolicy::casual(),
                    Some("free") | None => twanga_tabs::playback::PlaybackPolicy::FreePlay,
                    Some(other) => unreachable!("clap value_parser rejected '{other}'"),
                }
            };
            run_playback(
                path,
                parsed,
                tuning,
                transpose_mode,
                bpm,
                !no_metronome,
                policy,
                loop_spec,
                capo,
                pre_roll,
                resume_col,
                silence_rms,
                from_file,
            )?;
        }
        Command::Devices => {
            for name in twanga_audio::list_input_devices()? {
                println!("{name}");
            }
        }
        Command::Import { input, from, title } => import::run_import(input, from, title)?,
        Command::Convert { input, out, from } => import::run_convert(input, out, from)?,
        Command::Tunings { action } => match action {
            TuningsAction::List => run_tunings_list()?,
            TuningsAction::Path => run_tunings_path()?,
            TuningsAction::Add => run_tunings_add()?,
            TuningsAction::Remove { slug, force } => run_tunings_remove(slug, force)?,
        },
        Command::Patterns { action } => match action {
            None => run_patterns_picker()?,
            Some(PatternsAction::List) => run_patterns_list()?,
            Some(PatternsAction::Path) => run_patterns_path()?,
            Some(PatternsAction::Play {
                id,
                no_metronome,
                wait,
                no_loop,
                bpm,
            }) => run_patterns_play(id, no_metronome, wait, no_loop, bpm)?,
        },
        Command::Edit { path, out, action } => run_edit(path, out, action)?,
        Command::Docs { feature } => run_docs(feature)?,
        Command::Calibrate {
            show,
            round_trip,
            manual,
        } => run_calibrate(show, round_trip, manual)?,
    }
    Ok(())
}

/// `twanga calibrate` — dispatch to the right calibration flow.
/// `--show` prints the stored value; `--round-trip` skips the
/// wizard and runs the measurement directly; `--manual <ms>` skips
/// the wizard and saves a hand-entered value; bare invocation runs
/// the interactive wizard that picks the right method based on
/// the user's mic / output setup.
fn run_calibrate(show: bool, round_trip: bool, manual: Option<u32>) -> Result<()> {
    let Some(root) = twanga_paths::data_root() else {
        return Err(anyhow!(
            "couldn't resolve a data root (no home dir + no portable sentinel)"
        ));
    };
    let path = root.latency_path();

    if show {
        return calibrate_show(&path);
    }
    if round_trip {
        return calibrate_round_trip(&path);
    }
    if let Some(ms) = manual {
        return calibrate_manual_from_flag(&path, ms);
    }
    calibrate_wizard(&path)
}

fn calibrate_show(path: &Path) -> Result<()> {
    match calibration::LatencyCalibration::load(path)? {
        Some(cal) => {
            eprintln!("Device:       {}", cal.device_name);
            eprintln!(
                "Latency:      {} ms (via {})",
                cal.latency_ms,
                cal.method.label()
            );
            eprintln!("Measured at:  {}", cal.measured_at);
            eprintln!("Path:         {}", path.display());
        }
        None => {
            eprintln!("No calibration on disk at {}.", path.display());
            eprintln!("Run `twanga calibrate` (without --show) to measure.");
        }
    }
    Ok(())
}

fn calibrate_round_trip(path: &Path) -> Result<()> {
    eprintln!(
        "Calibrating output→input round-trip latency over {} clicks.",
        calibration::CLICK_COUNT
    );
    eprintln!("Have your mic positioned where you'd play. Stay quiet during the measurement.");
    eprintln!();
    let mut progress = |i: usize, total: usize| {
        eprint!("\rClick {i}/{total}…   ");
    };
    let cal = calibration::run_calibration(&mut progress)?;
    eprintln!("\r                       ");
    cal.save(path)?;
    eprintln!("Device:    {}", cal.device_name);
    eprintln!(
        "Latency:   {} ms (via {})",
        cal.latency_ms,
        cal.method.label()
    );
    eprintln!("Saved to:  {}", path.display());
    Ok(())
}

fn calibrate_manual_from_flag(path: &Path, ms: u32) -> Result<()> {
    if ms > 1000 {
        return Err(anyhow!(
            "manual latency {ms} ms is out of plausible range (max 1000)"
        ));
    }
    // Capture the device name by briefly opening a stream — same
    // invalidation key the round-trip path uses. If no mic is
    // available, fall back to a synthetic name so the calibration
    // is still applied (matches the headphones / no-mic case).
    let device_name = match twanga_audio::InputStream::open() {
        Ok(s) => s.device_name,
        Err(_) => "(no mic detected)".to_string(),
    };
    let cal = calibration::manual_calibration(device_name, ms);
    cal.save(path)?;
    eprintln!("Device:    {}", cal.device_name);
    eprintln!(
        "Latency:   {} ms (via {})",
        cal.latency_ms,
        cal.method.label()
    );
    eprintln!("Saved to:  {}", path.display());
    Ok(())
}

/// Interactive wizard. Asks two setup questions then dispatches to
/// the right measurement method:
///
///   Q1=mic + Q2=speakers   → round-trip (acoustic loop works)
///   anything else           → manual entry (with driver hints)
///
/// Aborts if stdin / stderr aren't TTYs — the wizard expects a
/// human typing answers. Scripts should pass `--round-trip` or
/// `--manual <ms>` explicitly.
fn calibrate_wizard(path: &Path) -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "interactive wizard requires a TTY. Use `--round-trip` or `--manual <ms>` for scripts."
        ));
    }

    eprintln!("Calibration wizard");
    eprintln!("──────────────────");
    eprintln!("Two questions to pick the right measurement for your setup.");
    eprintln!();

    let input = prompt_choice(
        "Q1. Where does TWANGA listen for your playing?",
        &[
            (
                "a",
                "Microphone — captures sound acoustically (USB mic, condenser, dynamic mic in front of an amp)",
            ),
            (
                "b",
                "Direct cable from instrument — USB instrument cable, or instrument cable into an audio interface (no acoustic capture)",
            ),
            ("c", "Nothing connected yet (set manually for now)"),
        ],
    )?;

    // Q2 only matters when the input is a microphone — that's the
    // only branch where round-trip is physically possible. Line-in
    // (no acoustic capture) and "nothing connected" (no input) both
    // skip Q2 and route straight to manual entry.
    let output = if input == "a" {
        prompt_choice(
            "Q2. How do you hear TWANGA's audio (metronome, count-in)?",
            &[
                ("a", "Speakers in the same room as the mic"),
                ("b", "Headphones (or speakers far from the mic)"),
                ("c", "No audible playback (visual cues only)"),
            ],
        )?
    } else {
        "skip".to_string()
    };

    match (input.as_str(), output.as_str()) {
        // Only mic + speakers supports round-trip — that's the one
        // case where TWANGA can physically capture its own click
        // through the air. Line-in users can't be captured back even
        // if their speakers work; they need manual entry.
        ("a", "a") => {
            eprintln!();
            // Output verification before the real measurement.
            // Catches "speakers muted / wrong output device" before
            // it presents as "no clicks detected" (which gives the
            // user no clue which side of the loop is broken). Only
            // runs under the wizard — `--round-trip` skips it for
            // scriptability.
            if !prompt_output_test()? {
                eprintln!();
                eprintln!("Audio output isn't reaching your speakers. Switching to manual entry.");
                eprintln!(
                    "(Tip: run `twanga tune` separately to verify your mic + pitch detection.)"
                );
                return calibrate_manual_wizard(path);
            }
            eprintln!();
            eprintln!("Setup detected: acoustic round-trip available. Running measurement.");
            calibrate_round_trip(path)
        }
        // Everything else: manual entry with a driver-default hint.
        _ => {
            eprintln!();
            eprintln!("Setup detected: acoustic round-trip not available. Setting manually.");
            eprintln!("(Tip: run `twanga tune` separately to verify your mic + pitch detection.)");
            calibrate_manual_wizard(path)
        }
    }
}

/// Play a single test click + ask whether the user heard it.
/// Returns `Ok(true)` on yes (proceed with round-trip), `Ok(false)`
/// on no (the wizard reroutes to manual). Wizard-only — the
/// `--round-trip` flag skips this and goes straight to measurement.
fn prompt_output_test() -> Result<bool> {
    use std::io::{BufRead, Write};
    eprintln!();
    eprintln!("Playing one test click — confirm you can hear it.");
    let mut output = twanga_audio::OutputStream::open()?;
    let click = metronome_click(output.sample_rate);
    output.write(&click);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let stderr = std::io::stderr();
    let stdin = std::io::stdin();
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();
    for _ in 0..5 {
        write!(stderr_lock, "Did you hear the click? [y/n]: ")?;
        stderr_lock.flush()?;
        line.clear();
        stdin_lock.read_line(&mut line)?;
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(stderr_lock, "invalid response — y or n")?,
        }
    }
    Err(anyhow!("too many invalid responses"))
}

/// Interactive manual-entry sub-flow. Shows typical-value
/// suggestions per detected platform so the user has a starting
/// point even if they don't know their interface's spec.
fn calibrate_manual_wizard(path: &Path) -> Result<()> {
    eprintln!();
    eprintln!("Typical values to pick from:");
    for (label, ms) in driver_default_hints() {
        eprintln!("  {ms:>3} ms — {label}");
    }
    eprintln!();

    let ms = prompt_u32("Enter latency in milliseconds (0–1000)", 0, 1000)?;
    calibrate_manual_from_flag(path, ms)
}

/// Platform-specific "typical" latency hints surfaced in the
/// manual-entry wizard. Wide ranges intentional — they're
/// suggestions, not measurements.
fn driver_default_hints() -> Vec<(&'static str, u32)> {
    let common = vec![
        ("ASIO / CoreAudio dedicated interface", 10),
        ("Built-in audio (WASAPI / CoreAudio / PulseAudio)", 30),
        ("Class-compliant USB mic", 40),
        ("Bluetooth audio (any direction)", 150),
    ];
    common
}

/// Prompt for a single-letter choice from a small menu. Re-prompts
/// on invalid input up to 5 times.
fn prompt_choice(question: &str, options: &[(&str, &str)]) -> Result<String> {
    use std::io::{BufRead, Write};
    eprintln!("{question}");
    for (key, label) in options {
        eprintln!("  [{key}] {label}");
    }
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();
    for _ in 0..5 {
        write!(stderr_lock, "> ")?;
        stderr_lock.flush()?;
        line.clear();
        stdin_lock.read_line(&mut line)?;
        let trimmed = line.trim().to_lowercase();
        if options.iter().any(|(k, _)| *k == trimmed) {
            return Ok(trimmed);
        }
        writeln!(
            stderr_lock,
            "invalid choice — type one of: {}",
            options
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    Err(anyhow!("too many invalid responses"))
}

/// Prompt for an integer in a closed range. Re-prompts up to 5
/// times.
fn prompt_u32(question: &str, min: u32, max: u32) -> Result<u32> {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    let mut stderr_lock = stderr.lock();
    let mut stdin_lock = stdin.lock();
    let mut line = String::new();
    for _ in 0..5 {
        write!(stderr_lock, "{question}: ")?;
        stderr_lock.flush()?;
        line.clear();
        stdin_lock.read_line(&mut line)?;
        let trimmed = line.trim();
        if let Ok(n) = trimmed.parse::<u32>()
            && (min..=max).contains(&n)
        {
            return Ok(n);
        }
        writeln!(stderr_lock, "invalid number — expected {min}..={max}")?;
    }
    Err(anyhow!("too many invalid responses"))
}

/// Look up the persisted hardware-latency value for the currently-
/// open input device. Returns the calibrated millisecond offset on
/// match, 0 on any other outcome (no calibration, stale calibration
/// from a different device, IO error reading the file). Prints a
/// status line to stderr in each case so the user always knows
/// what scoring assumptions are in play before plucking the first
/// note — the most common failure mode is "I calibrated once, then
/// switched USB cables, now my scores look weird."
fn resolve_hardware_latency(device_name: &str) -> u32 {
    let Some(root) = twanga_paths::data_root() else {
        return 0;
    };
    let path = root.latency_path();
    match calibration::LatencyCalibration::load(&path) {
        Ok(Some(cal)) if cal.applies_to(device_name) => {
            eprintln!(
                "Latency:    {} ms (calibrated for '{}')",
                cal.latency_ms, cal.device_name
            );
            cal.latency_ms
        }
        Ok(Some(cal)) => {
            eprintln!(
                "Latency:    uncalibrated for '{}' (saved value is for '{}'). \
                 Scoring may be off; run `twanga calibrate` to refresh.",
                device_name, cal.device_name
            );
            0
        }
        Ok(None) => {
            eprintln!("Latency:    uncalibrated. Run `twanga calibrate` for tighter scoring.");
            0
        }
        Err(e) => {
            eprintln!(
                "Latency:    couldn't read {} ({e}); treating as uncalibrated.",
                path.display()
            );
            0
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Docs
// ──────────────────────────────────────────────────────────────────────────
//
// Per-feature documentation embedded in the binary via `include_str!`. The
// same markdown files are bundled into the deployed web app at
// `assets/docs/<slug>.md` (CI step in `.github/workflows/pages.yml`) and
// rendered by the SPA's docs viewer (`#docs/<slug>`). Single source of
// truth: `docs/features/*.md` at the repo root.
//
// Rendering is intentionally not done here. Terminals vary in markdown
// support and we'd be reinventing `glow` / `mdcat` / `bat` poorly.
// Print the raw markdown to stdout; users who want fancy rendering pipe
// it through their tool of choice.

const DOC_TUNER: &str = include_str!("../../../docs/features/tuner.md");
const DOC_RECORDER: &str = include_str!("../../../docs/features/recorder.md");
const DOC_PLAYBACK: &str = include_str!("../../../docs/features/playback.md");
const DOC_PATTERNS: &str = include_str!("../../../docs/features/patterns.md");
const DOC_EDITOR: &str = include_str!("../../../docs/features/editor.md");
const DOC_TUNINGS: &str = include_str!("../../../docs/features/tunings.md");
const DOC_CALIBRATE: &str = include_str!("../../../docs/features/calibrate.md");
const DOC_HARDWARE: &str = include_str!("../../../docs/features/hardware.md");
const DOC_USER_GUIDE: &str = include_str!("../../../docs/features/user-guide.md");
const DOC_IMPORTER: &str = include_str!("../../../docs/features/importer.md");

/// Slug → embedded markdown body. Order here is the listing order shown
/// to the user when they run `twanga docs` with no arg.
const DOCS_PAGES: &[(&str, &str, &str)] = &[
    (
        "tuner",
        "Live pitch detection vs your chosen tuning.",
        DOC_TUNER,
    ),
    (
        "recorder",
        "Capture played notes as an alphaTex tab.",
        DOC_RECORDER,
    ),
    (
        "playback",
        "Play a tab with metronome, wait, loop, transpose.",
        DOC_PLAYBACK,
    ),
    ("patterns", "Bundled rhythm + picking drills.", DOC_PATTERNS),
    (
        "editor",
        "Post-capture cell-level edits to recordings.",
        DOC_EDITOR,
    ),
    (
        "importer",
        "Add alphaTex / MusicXML / MXL files to your library.",
        DOC_IMPORTER,
    ),
    (
        "tunings",
        "Built-in + user-defined tuning registry.",
        DOC_TUNINGS,
    ),
    (
        "calibrate",
        "Measure your audio chain's round-trip latency for accurate scoring.",
        DOC_CALIBRATE,
    ),
    (
        "hardware",
        "Audio-input setup guide — mics, instrument cables, interfaces.",
        DOC_HARDWARE,
    ),
    (
        "user-guide",
        "Paths + portable mode, audio architecture, privacy, credits.",
        DOC_USER_GUIDE,
    ),
];

/// Pure helper: format the listing shown by `twanga docs` (no arg).
/// Extracted so tests don't have to capture stdout. The `run_docs`
/// wrapper handles the println side-effect.
fn docs_listing_text() -> String {
    let mut out = String::new();
    out.push_str("Per-feature documentation embedded in this binary.\n\n");
    out.push_str("Usage: twanga docs <feature>\n\n");
    out.push_str("Available pages:\n");
    for (slug, blurb, _) in DOCS_PAGES {
        out.push_str(&format!("  {slug:<10} {blurb}\n"));
    }
    out.push_str("\nMarkdown is printed raw to stdout. Pipe through your\n");
    out.push_str("renderer of choice for prettier output, e.g.:\n");
    out.push_str("  twanga docs playback | glow -\n");
    out.push_str("  twanga docs playback | bat -l md\n");
    out
}

/// Pure helper: look up the embedded markdown body for `slug`. Returns
/// `Err` with a human-readable message when no page matches (so the
/// surface error in `run_docs` and any future programmatic consumer
/// produce the same text).
fn docs_page_text(slug: &str) -> Result<&'static str> {
    let normalised = slug.to_lowercase();
    DOCS_PAGES
        .iter()
        .find(|(s, _, _)| *s == normalised)
        .map(|(_, _, body)| *body)
        .ok_or_else(|| anyhow!("unknown docs page '{slug}' — try `twanga docs` for the list"))
}

fn run_docs(feature: Option<String>) -> Result<()> {
    let Some(slug) = feature else {
        print!("{}", docs_listing_text());
        return Ok(());
    };
    print!("{}", docs_page_text(&slug)?);
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// Bundled content picker (`twanga play` with no path)
// ──────────────────────────────────────────────────────────────────────────

/// Build a labelled list of choices for the bare `twanga play` picker.
/// Pairs each menu line with the filesystem path it resolves to so the
/// caller can hand it off to `run_playback` without re-discovery.
struct PlayChoice {
    label: String,
    path: PathBuf,
}

fn collect_play_choices() -> Vec<PlayChoice> {
    let mut choices: Vec<PlayChoice> = Vec::new();

    // Bundled examples first — they're stable across runs and don't
    // depend on the user having recorded anything.
    if let Ok(examples) = bundled::load_examples() {
        for entry in examples {
            if let Ok(path) = bundled::resolve_example_path(&entry) {
                let tuning = entry.tuning.as_deref().unwrap_or("");
                let label = if tuning.is_empty() {
                    format!("[example] {}", entry.title)
                } else {
                    format!("[example] {} ({tuning})", entry.title)
                };
                choices.push(PlayChoice { label, path });
            }
        }
    }

    // Then bundled patterns — flatten the group tree, sort by
    // difficulty within each group (the same ordering the GUI uses).
    if let Ok(groups) = bundled::load_patterns() {
        for (group, pattern) in bundled::flat_pattern_list(&groups) {
            if let Ok(path) = bundled::resolve_pattern_path(&pattern) {
                let pips = bundled::difficulty_pips(pattern.difficulty);
                let prefix = if pips.is_empty() {
                    format!("[pattern · {}]", group.title)
                } else {
                    format!("[pattern · {} · {pips}]", group.title)
                };
                choices.push(PlayChoice {
                    label: format!("{prefix} {}", pattern.title),
                    path,
                });
            }
        }
    }

    // User-imported tabs next — distinct from recordings (those are
    // live captures from this machine; these arrived from external
    // files via the Importer or `twanga import`). Most-recent first
    // matches the recordings ordering below.
    if let Ok(imports) = bundled::scan_library() {
        for tab in imports {
            let stem = tab
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("tab");
            choices.push(PlayChoice {
                label: format!("[imported] {} ({stem})", tab.title),
                path: tab.path,
            });
        }
    }

    // Finally any local recordings — those are the user's own takes,
    // most-recent first so the natural use case ("play back what I
    // just recorded") works without scrolling.
    if let Ok(recordings) = bundled::scan_recordings() {
        for rec in recordings {
            let stem = rec
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("recording");
            choices.push(PlayChoice {
                label: format!("[recording] {} ({stem})", rec.title),
                path: rec.path,
            });
        }
    }

    choices
}

/// Show an interactive picker that lets the user pick a file to play
/// when `twanga play` was invoked with no path. Returns `None` if
/// nothing's available to play — caller should print a hint and exit.
fn prompt_play_target() -> Result<Option<PathBuf>> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "no path given and stdin isn't a TTY; pass a `.alphatex` file path or run interactively"
        ));
    }
    let choices = collect_play_choices();
    if choices.is_empty() {
        eprintln!(
            "No alphaTex files found. Looked in:\n  {}\n  {}\n  {}/\n  {}/",
            bundled::examples_manifest_path().display(),
            bundled::patterns_manifest_path().display(),
            bundled::library_dir_path().display(),
            bundled::recordings_dir_path().display(),
        );
        eprintln!("Pass a `.alphatex` path explicitly (e.g. `twanga play path/to/file.alphatex`),");
        eprintln!("or run from a directory that has one of the above.");
        return Ok(None);
    }
    let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
    let idx = twanga_tui::select_with_default_and_hint(
        "Choose a tab to play:",
        &labels,
        0,
        Some("tip: pass a `.alphatex` path directly to skip this menu"),
    )?;
    Ok(Some(choices[idx].path.clone()))
}

// ──────────────────────────────────────────────────────────────────────────
// `twanga patterns` subcommand handlers
// ──────────────────────────────────────────────────────────────────────────

/// `twanga patterns` (no subcommand) — interactive picker over the
/// bundled patterns. After the user picks one, plays it through the
/// same `run_playback` path `twanga play` uses, with `--loop full`
/// defaulted (patterns are meant to loop).
fn run_patterns_picker() -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(anyhow!(
            "interactive picker requires a TTY — use `twanga patterns play <id>` non-interactively"
        ));
    }
    let groups = bundled::load_patterns()?;
    let flat = bundled::flat_pattern_list(&groups);
    if flat.is_empty() {
        eprintln!(
            "No patterns found at {}.",
            bundled::patterns_manifest_path().display()
        );
        eprintln!("Run from the TWANGA repo root, or use `twanga play <path>` instead.");
        return Ok(());
    }
    let labels: Vec<String> = flat
        .iter()
        .map(|(group, pattern)| {
            let pips = bundled::difficulty_pips(pattern.difficulty);
            if pips.is_empty() {
                format!("{} · {}", group.title, pattern.title)
            } else {
                format!("{pips}  {} · {}", group.title, pattern.title)
            }
        })
        .collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let idx = twanga_tui::select_with_default_and_hint(
        "Choose a pattern to practice:",
        &refs,
        0,
        Some("patterns loop by default — slow the BPM until you can play it cleanly"),
    )?;
    let (_, pattern) = &flat[idx];
    run_patterns_play(
        pattern.id.clone(),
        /* no_metronome */ false,
        /* wait */ false,
        /* no_loop */ false,
        /* bpm */ None,
    )
}

/// `twanga patterns list` — catalog view. Prints each group with its
/// description and then the patterns sorted by difficulty, marked
/// with ★ pips. Same ordering the GUI's Patterns screen renders.
fn run_patterns_list() -> Result<()> {
    let groups = bundled::load_patterns()?;
    if groups.is_empty() {
        println!(
            "(no patterns manifest at {})",
            bundled::patterns_manifest_path().display()
        );
        return Ok(());
    }
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{}", group.title);
        if let Some(desc) = &group.description {
            println!("  {desc}");
        }
        let mut sorted = group.patterns.clone();
        sorted.sort_by_key(|p| p.difficulty.unwrap_or(u32::MAX));
        for pattern in sorted {
            let pips = bundled::difficulty_pips(pattern.difficulty);
            let tuning = pattern.tuning.as_deref().unwrap_or("");
            let pad = if pips.is_empty() {
                "       ".to_string()
            } else {
                pips
            };
            if tuning.is_empty() {
                println!("  {pad}  {}  ({})", pattern.title, pattern.id);
            } else {
                println!(
                    "  {pad}  {}  ({}, tuning: {tuning})",
                    pattern.title, pattern.id
                );
            }
        }
    }
    Ok(())
}

/// `twanga patterns path` — print the manifest path. Companion to
/// `twanga tunings path`.
fn run_patterns_path() -> Result<()> {
    println!("{}", bundled::patterns_manifest_path().display());
    Ok(())
}

/// `twanga patterns play <id>` — non-interactive: resolve the id in
/// the manifest, hand it off to `run_playback`. Defaults to looping
/// (the whole point of a pattern); `--no-loop` flips that.
fn run_patterns_play(
    id: String,
    no_metronome: bool,
    wait: bool,
    no_loop: bool,
    bpm_arg: Option<String>,
) -> Result<()> {
    let groups = bundled::load_patterns()?;
    let mut found: Option<(bundled::PatternGroup, bundled::PatternEntry)> = None;
    for group in &groups {
        for pattern in &group.patterns {
            if pattern.id == id {
                found = Some((group.clone(), pattern.clone()));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some((_, pattern)) = found else {
        return Err(anyhow!(
            "no pattern with id '{id}'. Run `twanga patterns list` to see what's available."
        ));
    };
    let path = bundled::resolve_pattern_path(&pattern)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;
    let parsed = alphatex::parse(&content).map_err(|e| anyhow!("failed to parse alphaTex: {e}"))?;
    if parsed.columns.is_empty() {
        return Err(anyhow!("pattern '{}' has no notes to play", id));
    }
    let bpm = resolve_bpm_override(bpm_arg)?;
    let loop_spec = if no_loop {
        None
    } else {
        Some("full".to_string())
    };
    // Patterns play stays on the simpler `wait → policy` mapping —
    // `twanga patterns play` doesn't take `--policy` yet (it's a v2-of-v2
    // surface, gated on someone wanting scoring on drill exercises).
    let policy = if wait {
        twanga_tabs::playback::PlaybackPolicy::wait()
    } else {
        twanga_tabs::playback::PlaybackPolicy::FreePlay
    };
    run_playback(
        path,
        parsed,
        /* tuning_override */ None,
        alphatex::TransposeMode::Drop,
        bpm,
        !no_metronome,
        policy,
        loop_spec,
        /* capo_spec */ None,
        /* pre_roll */ 4,
        /* resume_col */ None,
        /* silence_rms */ None,
        /* from_file */ None,
    )
}

// ──────────────────────────────────────────────────────────────────────────
// `twanga edit` — non-interactive tab mutator
// ──────────────────────────────────────────────────────────────────────────

/// Apply a single edit action to an `.alphatex` file and write the
/// result back (in place, or to `out` if specified). One operation
/// per invocation — chain in a shell script for batch edits.
///
/// Each action goes through the same `serialize_recording`-style
/// path the Recorder + GUI Editor use: parse → mutate `ParsedTab`
/// in memory → re-serialize via `AlphaTexWriter`. So saved output is
/// bit-for-bit indistinguishable from a fresh recording.
fn run_edit(path: PathBuf, out: Option<PathBuf>, action: EditAction) -> Result<()> {
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;
    let mut parsed =
        alphatex::parse(&content).map_err(|e| anyhow!("failed to parse alphaTex: {e}"))?;
    // Construct the working `Tuning` from the file's `\tuning` header,
    // then override its `name` with the file's original subtitle so
    // the AlphaTexWriter round-trips the subtitle line instead of
    // clobbering it with the placeholder "(from alphaTex)". The
    // subtitle's `; capo=...` suffix is stripped (we pass capo
    // separately to the writer, which re-attaches it on the way out).
    let mut tuning = parsed
        .tuning()
        .ok_or_else(|| anyhow!("'{}' has no parseable \\tuning header", path.display()))?;
    if let Some(human_subtitle) = parsed.subtitle_display() {
        tuning.name = human_subtitle;
    }
    let string_count = tuning.strings.len();

    match action {
        EditAction::Set {
            column,
            string,
            fret,
        } => {
            validate_column(column, parsed.columns.len())?;
            validate_string(string, string_count)?;
            let col = &mut parsed.columns[column];
            // Drop any existing hit on this string, then push the new one.
            col.hits.retain(|(s, _)| *s != string);
            col.hits.push((string, fret));
            // Keep hits sorted by string for stable output ordering — the
            // recorder writes them in string-index order too.
            col.hits.sort_by_key(|(s, _)| *s);
            eprintln!("set col {column}, string {string}, fret {fret}");
        }
        EditAction::Clear { column, string } => {
            validate_column(column, parsed.columns.len())?;
            validate_string(string, string_count)?;
            let col = &mut parsed.columns[column];
            let before = col.hits.len();
            col.hits.retain(|(s, _)| *s != string);
            if col.hits.len() == before {
                eprintln!("(col {column}, string {string} was already empty)");
            } else {
                eprintln!("cleared col {column}, string {string}");
            }
        }
        EditAction::ClearCol { column } => {
            validate_column(column, parsed.columns.len())?;
            parsed.columns[column].hits.clear();
            eprintln!("cleared col {column}");
        }
        EditAction::InsertCol { after } => {
            // `after = None` → append at the end. `after = Some(n)` →
            // insert after column n (resulting position = n+1).
            let total = parsed.columns.len();
            let insert_at = match after {
                Some(n) => {
                    if n >= total {
                        return Err(anyhow!(
                            "--after {n} is out of range (file has {total} columns; 0..{total} valid)"
                        ));
                    }
                    n + 1
                }
                None => total,
            };
            // Inherit duration_denom from the existing first column so
            // the new blank stays in time with the rest of the file.
            let duration_denom = parsed
                .columns
                .first()
                .map(|c| c.duration_denom)
                .unwrap_or(8);
            parsed.columns.insert(
                insert_at,
                alphatex::TabColumn {
                    duration_denom,
                    hits: Vec::new(),
                    articulation: None,
                },
            );
            eprintln!("inserted blank column at position {insert_at}");
        }
        EditAction::DeleteCol { column } => {
            validate_column(column, parsed.columns.len())?;
            parsed.columns.remove(column);
            eprintln!(
                "deleted col {column} (file now has {} columns)",
                parsed.columns.len()
            );
        }
        EditAction::Title { text } => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                parsed.title = None;
                eprintln!("cleared \\title");
            } else {
                parsed.title = Some(trimmed.to_string());
                eprintln!("set \\title to \"{trimmed}\"");
            }
        }
        EditAction::Bpm { bpm } => {
            validate_bpm(bpm)?;
            parsed.tempo = bpm;
            eprintln!("set \\tempo to {bpm}");
        }
    }

    // Re-serialize through AlphaTexWriter — same path as the Recorder's
    // save. Preserves tuning, capo (round-tripped via subtitle), title,
    // and resolution.
    let capo = parsed.capo().unwrap_or_else(|| Capo::none(string_count));
    let resolution_denom = parsed
        .columns
        .first()
        .map(|c| c.duration_denom)
        .unwrap_or(8);
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = AlphaTexWriter::new(
            &mut buf,
            &tuning,
            &capo,
            parsed.tempo,
            resolution_denom,
            parsed.title.as_deref(),
        )
        .map_err(|e| anyhow!("writer init failed: {e}"))?;
        // Each column gets serialized as the per-string fret marks. The
        // writer takes `&[Option<u8>]` — one entry per string, in
        // tuning order.
        let mut row: Vec<Option<u8>> = vec![None; string_count];
        for col in &parsed.columns {
            row.iter_mut().for_each(|m| *m = None);
            for (string, fret) in &col.hits {
                let idx = (*string as usize).saturating_sub(1);
                if idx < string_count {
                    row[idx] = Some(*fret);
                }
            }
            w.write_column(&row)
                .map_err(|e| anyhow!("write column: {e}"))?;
        }
        w.finalize().map_err(|e| anyhow!("finalize: {e}"))?;
    }
    let out_path = out.unwrap_or(path);
    fs::write(&out_path, buf)
        .with_context(|| format!("failed to write '{}'", out_path.display()))?;
    eprintln!("wrote {}", out_path.display());
    Ok(())
}

fn validate_column(col: usize, total: usize) -> Result<()> {
    if col >= total {
        Err(anyhow!(
            "column {col} is out of range (file has {total} columns; 0..{total} valid)"
        ))
    } else {
        Ok(())
    }
}

fn validate_string(string: u8, total: usize) -> Result<()> {
    if string == 0 || (string as usize) > total {
        Err(anyhow!(
            "string {string} is out of range (tuning has {total} strings; 1..={total} valid)"
        ))
    } else {
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Playback
// ──────────────────────────────────────────────────────────────────────────

/// Width (in tab columns) of the scrolling playback view.
const PLAYBACK_WINDOW_COLS: usize = 24;
/// Tolerance for wait-mode note matching, in cents.
const WAIT_MATCH_CENTS: f32 = 50.0;

/// The mic-or-file source plus its `Tuner`, paired so the playback
/// loop can feed samples through pitch detection in lockstep. `None`
/// when the chosen policy doesn't need audio (FreePlay).
type InputState = Option<(Box<dyn audio_source::SampleSource>, Tuner)>;

// 8 args is one over clippy's default ceiling. The arguments are
// independent user-CLI-flag values aggregated for the playback flow;
// bundling them into a struct just to satisfy the lint is more
// ceremony than clarity at this size.
#[allow(clippy::too_many_arguments)]
fn run_playback(
    path: PathBuf,
    parsed: ParsedTab,
    tuning_override: Option<String>,
    transpose_mode: alphatex::TransposeMode,
    bpm_override: Option<u32>,
    metronome: bool,
    policy: twanga_tabs::playback::PlaybackPolicy,
    loop_spec: Option<String>,
    capo_spec: Option<String>,
    pre_roll: u32,
    resume_col: Option<u64>,
    silence_rms: Option<f32>,
    from_file: Option<PathBuf>,
) -> Result<()> {
    // Local boolean derived from the policy for spots that still read
    // "is this wait mode?" in the obvious way — the previous code
    // threaded `wait: bool` deep into the loop and the tuner-init
    // branch, and keeping a local alias minimises churn while we
    // grow the policy surface.
    let wait = matches!(
        policy,
        twanga_tabs::playback::PlaybackPolicy::WaitOnPitch { .. }
    );
    // Does this policy need mic input at all? Wait mode does (pitch
    // matching) and ProximityScore does (onset collection); FreePlay
    // doesn't open a stream so the user can play at-tempo with no
    // audio routing.
    let needs_audio = !matches!(policy, twanga_tabs::playback::PlaybackPolicy::FreePlay);
    // Transpose if --tuning was provided (either explicitly via flag or via
    // the prompt). The transposed tab carries the target tuning's names in
    // its header, so downstream code (wait mode, display) sees one consistent
    // tuning throughout. We use the *with-report* variant so out-of-range
    // notes get surfaced to the user up front rather than silently
    // disappearing — they get a "Skipped notes" preamble before the cursor
    // starts and can hit `q` if it's worse than they expected.
    // `target_tuning` carries the *transpose target* tuning when one was
    // selected — including per-string `fret_offset` metadata that the
    // alphaTex `\tuning` header can't represent (it stores note names
    // only). Without this preserved copy, downstream code that derives
    // pitches from `tab.tuning_names` (`note_at_playhead`,
    // `matches_any_expected`) would compute `open + 7 = D5` for the
    // banjo drone at fret 7 instead of the correct `open + 2 = A4`.
    // `None` when no transpose happened — matches every legacy recording.
    let (tab, dropped, target_tuning) = if let Some(name) = tuning_override.as_deref() {
        let target = lookup_tuning(name).ok_or_else(|| {
            anyhow!(
                "unknown tuning preset '{name}'. options: {}",
                known_slugs().join(", ")
            )
        })?;
        let (transposed, dropped) =
            parsed.transpose_to_with_mode(&target, MAX_FRET, transpose_mode);
        (transposed, dropped, Some(target))
    } else {
        (parsed, Vec::new(), None)
    };
    let target_fret_offsets: Vec<u8> = match &target_tuning {
        Some(t) => t.strings.iter().map(|s| s.fret_offset).collect(),
        None => vec![0_u8; tab.tuning_names.len()],
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
    // fret 0 means "the open string above the capo." Prefer the preserved
    // `target_tuning` (when a transpose ran) over `tab.tuning()` so per-string
    // `fret_offset` metadata survives — alphaTex's `\tuning` header doesn't
    // carry it, and without it wait-mode would expect the wrong pitch on a
    // banjo drone fretted above its peg.
    // Effective tuning used for ALL pitch-match comparisons during
    // the session (wait-mode advance check + proximity-score onset
    // pairing). Constructed once when audio is needed; rebuilt from
    // the `target_tuning` (if a transpose ran — preserves
    // `fret_offset` metadata the alphaTex header can't carry) with
    // the `effective_capo` baked in so fret 0 means "the open
    // string above the capo."
    let tuning_for_pitch_match: Option<Tuning> = if needs_audio {
        let base = target_tuning.clone().map(Ok).unwrap_or_else(|| {
            tab.tuning()
                .ok_or_else(|| anyhow!("'\\tuning' header is missing or unparseable"))
        })?;
        let c = effective_capo
            .clone()
            .unwrap_or_else(|| Capo::none(base.strings.len()));
        Some(c.apply(&base).map_err(|e| anyhow!("{e}"))?)
    } else {
        None
    };
    let header_capo = effective_capo;

    let mut output = if metronome || pre_roll > 0 {
        Some(OutputStream::open()?)
    } else {
        None
    };
    let click = output.as_ref().map(|o| metronome_click(o.sample_rate));

    // Hardware round-trip latency from a prior `twanga calibrate`
    // run. Populated below if we open a live mic AND the saved
    // calibration's device matches; 0 in every other case (first
    // run, WAV-file replay, stale calibration after a device swap).
    let mut extra_latency_ms: u32 = 0;

    let (mut input_state, mut input_buf): (InputState, Vec<f32>) = if needs_audio {
        // Either a live mic or a paced WAV-file replay, depending on
        // `--from-file`. Both implement the narrow `SampleSource`
        // trait — the playback loop doesn't care which it has.
        let source: Box<dyn audio_source::SampleSource> = match from_file.as_deref() {
            Some(path) => Box::new(audio_source::WavSampleSource::from_file(path)?),
            None => {
                let stream = InputStream::open()?;
                // Live mic — resolve the persisted calibration. WAV
                // replay skips this because synth audio has no
                // hardware round-trip to compensate for.
                extra_latency_ms = resolve_hardware_latency(&stream.device_name);
                Box::new(stream)
            }
        };
        let sr = source.sample_rate();
        let mut tuner = Tuner::new(TunerMode::Chromatic, sr);
        // Both wait-mode and proximity-score depend on the silence
        // threshold being correct — a too-high default suppresses
        // legitimately quiet plucks; a too-low one fires YIN on
        // cable hum. Auto-calibrate at session start unless the
        // user pinned a value with `--silence-rms`. The WAV-file
        // path also calibrates so synth fixtures with realistic
        // noise floors behave the same as a live take.
        if let Some(rms) = silence_rms {
            tuner.set_silence_rms(rms);
        } else {
            tuner.start_noise_calibration(NOISE_CALIBRATION_SECONDS);
        }
        (Some((source, tuner)), vec![0.0_f32; READ_CHUNK])
    } else {
        (None, Vec::new())
    };

    // Per-string capo offset annotation suffix for each row label
    // (e.g. " +3" for a capo at fret 3). Empty string for strings
    // not affected by the capo. Computed once up-front because the
    // capo doesn't change during a playback session.
    let capo_suffixes: Vec<String> = (0..tab.tuning_names.len())
        .map(|i| {
            let off = header_capo
                .as_ref()
                .and_then(|c| c.offsets.get(i).copied())
                .unwrap_or(0);
            if off > 0 {
                format!(" +{off}")
            } else {
                String::new()
            }
        })
        .collect();
    let labels: Vec<String> = tab
        .tuning_names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{name}{}", capo_suffixes[i]))
        .collect();
    let name_width = labels.iter().map(|l| l.len()).max().unwrap_or(0);

    // Per-string open MIDI (post-capo) so `render_playback_rows` can
    // derive the absolute note name for the fret at the playhead
    // column. Parsed from the tab's tuning_names + the resolved capo
    // offsets so labels and notes stay in sync.
    let open_midis: Vec<MidiNote> = tab
        .tuning_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let base = MidiNote::from_name(name).unwrap_or(MidiNote(0));
            // Capo offsets are i32 (Capo allows reentrant-string drops
            // in theory) but in practice for the live-note display
            // we only care about positive offsets; clamp to [0, 127]
            // and saturating_add into u8.
            let off: u8 = header_capo
                .as_ref()
                .and_then(|c| c.offsets.get(i).copied())
                .map(|o| o.clamp(0, 127) as u8)
                .unwrap_or(0);
            MidiNote(base.0.saturating_add(off))
        })
        .collect();

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
        let mode_suffix = match transpose_mode {
            alphatex::TransposeMode::OctaveShift => "  [octave-shift]",
            alphatex::TransposeMode::Drop => "",
        };
        eprintln!(
            "Transposed: {name} ({}){mode_suffix}",
            tab.tuning_names.join(" ")
        );
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
    eprintln!("Pre-roll:   {pre_roll}");
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
    eprintln!("  Controls: 'q' stop · 'p' pause/resume · '[' / ']' silence ∓ 6 dB (wait mode)");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let stdin_rx = twanga_tui::spawn_line_reader();
    // One row per string + one for the position/progress line.
    let mut display = MultiLineDisplay::new(tab.tuning_names.len() + 1);
    let mut paused = false;

    if run_pre_roll(pre_roll, bpm, &mut output, click.as_ref(), &stdin_rx)? {
        eprintln!("Playback cancelled during pre-roll.");
        return Ok(());
    }

    // Proximity-score state — only populated when `policy` is
    // `ProximityScore`. The clock starts AFTER pre-roll so column
    // expected_ms values + collected onset timestamps share the
    // same zero. Events accumulate across the whole session;
    // looping sessions get scored from the first iteration's
    // schedule, not per-iteration (a planned per-iteration view
    // lives in the BACKLOG).
    let is_scoring = matches!(
        policy,
        twanga_tabs::playback::PlaybackPolicy::ProximityScore { .. }
    );
    let playback_clock_origin = std::time::Instant::now();
    let mut onset_events: Vec<twanga_tabs::playback::OnsetEvent> = Vec::new();

    // Track the current column outside the inner loops so the
    // user-initiated stop paths can save a resume bookmark before
    // returning. Updated on every column tick; the initial value
    // only matters if we exit before the inner loop even starts
    // (e.g. degenerate empty range — handled upstream, but defensive).
    #[allow(unused_assignments)]
    let mut last_col_idx: usize = loop_start;
    let mut first_iter = true;
    let save_bookmark_and_exit = |path: &Path,
                                  parsed_title: Option<&str>,
                                  col: usize|
     -> Result<()> {
        // Best-effort — bookmark failure shouldn't blow up the stop.
        if let Err(e) = play_resume::record(path, col as u64, parsed_title.map(|s| s.to_string())) {
            eprintln!("(couldn't save resume bookmark: {e})");
        }
        Ok(())
    };
    'session: loop {
        let mut col_idx = if first_iter {
            first_iter = false;
            resume_col
                .map(|c| (c as usize).min(loop_end.saturating_sub(1)))
                .unwrap_or(loop_start)
        } else {
            loop_start
        };
        while col_idx < loop_end {
            last_col_idx = col_idx;
            if twanga_tui::is_shutdown_requested() {
                eprintln!();
                eprintln!("Playback stopped.");
                save_bookmark_and_exit(&path, tab.title.as_deref(), last_col_idx)?;
                return Ok(());
            }
            if let Ok(input) = stdin_rx.try_recv() {
                if is_quit_input(&input) {
                    eprintln!();
                    eprintln!("Playback stopped.");
                    save_bookmark_and_exit(&path, tab.title.as_deref(), last_col_idx)?;
                    return Ok(());
                }
                if is_pause_input(&input) {
                    paused = !paused;
                    if paused {
                        eprintln!("\n[paused — press 'p' + Enter to resume]");
                    } else {
                        eprintln!("[resuming]");
                    }
                } else if let Some((_, ref mut wait_tuner)) = input_state {
                    // Threshold-step keys only do anything when there's
                    // a tuner to step (i.e. wait mode is on; without
                    // `--wait`, the input_state is None and the
                    // keypress is a no-op).
                    if is_threshold_down(&input) {
                        step_silence_threshold(wait_tuner, false);
                    } else if is_threshold_up(&input) {
                        step_silence_threshold(wait_tuner, true);
                    }
                }
            }
            if paused {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue; // don't advance col_idx
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
                &labels,
                &open_midis,
                &target_fret_offsets,
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

            // Per-column tick. Three branches by policy:
            //   1. wait mode + non-rest column → block until matched.
            //   2. proximity-score mode → sleep ms_per_col while
            //      simultaneously reading the mic + recording any
            //      onset-tagged readings with their wall-clock
            //      timestamp (scored against the schedule at session
            //      end).
            //   3. free-play / wait-mode rest → simple sleep.
            if wait && !column.hits.is_empty() {
                let tuning = tuning_for_pitch_match
                    .as_ref()
                    .expect("wait mode requires tuning");
                wait_for_expected_note(
                    &mut input_state,
                    &mut input_buf,
                    &column.hits,
                    tuning,
                    &stdin_rx,
                )?;
            } else if is_scoring {
                capture_onsets_for_duration(
                    &mut input_state,
                    &mut input_buf,
                    ms_per_col,
                    playback_clock_origin,
                    &mut onset_events,
                    extra_latency_ms,
                );
            } else {
                std::thread::sleep(std::time::Duration::from_millis(ms_per_col as u64));
            }
            col_idx += 1;
        }
        if !repeat {
            break 'session;
        }
    }

    eprintln!();
    eprintln!("Playback finished.");

    // Score the session if we were in proximity-score mode. Build the
    // expected schedule from the loop range we just played, pair
    // detected onsets against expected columns via
    // `twanga_tabs::playback::score`, and surface the aggregate. Per-
    // column outcomes are dropped here — the BACKLOG-tracked "replay
    // mistakes" feature is the natural home for keeping them.
    if is_scoring {
        // `needs_audio` was true, so `tuning_for_pitch_match` is
        // populated — unwrap is safe by construction.
        let tuning = tuning_for_pitch_match
            .as_ref()
            .expect("proximity-score policy requires tuning");
        let played_columns = &tab.columns[loop_start..loop_end];
        let schedule = twanga_tabs::playback::build_schedule(played_columns, bpm);
        let outcomes = twanga_tabs::playback::score(&schedule, &onset_events, policy, tuning);
        let summary = twanga_tabs::playback::PlaybackSummary::from_outcomes(&outcomes);
        print_score_summary(&summary);
    }

    Ok(())
}

/// Capture mic samples for `duration_ms` while accumulating onset-
/// tagged tuner readings as [`OnsetEvent`]s. Used by proximity-score
/// playback to collect what the user actually played alongside the
/// tempo-driven playhead — the resulting events get scored against
/// the expected schedule at session end.
///
/// Timestamps are measured from `clock_origin` so they share a zero
/// with `build_schedule`'s `expected_ms` values (the caller passes
/// the post-pre-roll Instant). When `input_state` is `None` (no mic
/// session opened — wouldn't normally happen in score mode but kept
/// defensive) the call degrades to a plain sleep so the tempo still
/// holds.
fn capture_onsets_for_duration(
    input_state: &mut InputState,
    buf: &mut [f32],
    duration_ms: u32,
    clock_origin: std::time::Instant,
    events: &mut Vec<twanga_tabs::playback::OnsetEvent>,
    extra_latency_ms: u32,
) {
    let Some((stream, tuner)) = input_state.as_mut() else {
        std::thread::sleep(std::time::Duration::from_millis(duration_ms as u64));
        return;
    };
    // Two latency sources stack:
    //
    //   1. `Tuner::window_latency_ms()` — DSP pipeline delay (YIN
    //      analysis window length, ~170 ms at 8192/48k).
    //   2. `extra_latency_ms` — hardware round-trip from a prior
    //      `twanga calibrate` run, or 0 if uncalibrated.
    //
    // Subtracting both from the wall-clock onset timestamp recovers
    // "when did the user pluck" rather than "when did we confirm
    // the pitch and when did the audio chain get round-tripped
    // through driver buffers." Without it, on-time plucks under
    // tight policy systematically score as Late.
    let latency_ms = tuner.window_latency_ms() + extra_latency_ms;
    let until = std::time::Instant::now() + std::time::Duration::from_millis(duration_ms as u64);
    while std::time::Instant::now() < until {
        let n = stream.read(buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            let now_ms = clock_origin.elapsed().as_millis() as u32;
            for r in tuner.take_readings() {
                if r.from_onset_window {
                    events.push(twanga_tabs::playback::OnsetEvent {
                        timestamp_ms: now_ms.saturating_sub(latency_ms),
                        detected_hz: r.detected.hz(),
                    });
                }
            }
        } else {
            // No samples queued yet — yield briefly to avoid spinning
            // the CPU while still respecting the column deadline.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// Pretty-print a proximity-score `PlaybackSummary` to stderr after
/// the playback loop finishes. Format chosen to match the rest of
/// the CLI's session-end output style (single block, key: value rows,
/// no trailing newline noise). Percentages render against the played
/// total so a 5-note song gives a useful breakdown rather than
/// rounding-to-100% on every line.
fn print_score_summary(summary: &twanga_tabs::playback::PlaybackSummary) {
    let total = summary.total();
    if total == 0 {
        // No non-rest columns in the played range — nothing to score.
        // Quiet exit; the user already saw "Playback finished."
        return;
    }
    eprintln!();
    eprintln!("Score:");
    let pct = |n: usize| (n as f32 / total as f32 * 100.0).round() as u32;
    eprintln!("  Hit:         {} ({}%)", summary.hit, pct(summary.hit));
    eprintln!("  Late:        {} ({}%)", summary.late, pct(summary.late));
    eprintln!(
        "  Missed:      {} ({}%)",
        summary.missed,
        pct(summary.missed)
    );
    eprintln!(
        "  Wrong pitch: {} ({}%)",
        summary.wrong_pitch,
        pct(summary.wrong_pitch)
    );
    eprintln!("  Total notes: {total}");
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
// Render-function args are inherently coupled (tab + view state +
// timing for the status line). Splitting into a struct would just
// move the noise around.
#[allow(clippy::too_many_arguments)]
fn render_playback_rows(
    tab: &ParsedTab,
    labels: &[String],
    open_midis: &[MidiNote],
    fret_offsets: &[u8],
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
    for (string_idx, label) in labels.iter().enumerate() {
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
        let padded = format!("{:<width$}", label, width = name_width);
        // Live-note cell: absolute note name (e.g. "B", "F#") for the
        // fret being played on this string at the playhead column.
        // Empty when this string isn't playing in the current column.
        // 2-char-max fixed width keeps the tab-body alignment stable.
        let note_cell = note_at_playhead(tab, string_idx, current_col, open_midis, fret_offsets);
        rows.push(format!("{padded} | {note_cell} | {content}"));
    }

    let pad = format!("{:<width$}", "", width = name_width);
    rows.push(format!(
        "{pad}        col {}/{}  (bar {}, beat {})  {} / {}",
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

/// Pitch-class name (e.g. "C#", "F", "B") for the fret being played on
/// `string_idx` at `current_col`. Two-char-wide left-padded so the
/// tab-body column stays aligned even when the cell is empty.
fn note_at_playhead(
    tab: &ParsedTab,
    string_idx: usize,
    current_col: usize,
    open_midis: &[MidiNote],
    fret_offsets: &[u8],
) -> String {
    let column = tab.columns.get(current_col);
    let string_num = (string_idx + 1) as u8;
    let fret = column.and_then(|c| {
        c.hits
            .iter()
            .find(|(s, _)| *s == string_num)
            .map(|(_, f)| *f)
    });
    match (fret, open_midis.get(string_idx)) {
        (Some(fret), Some(open)) => {
            // pitch = open + max(0, fret - fret_offset). On the banjo
            // drone (offset 5), a displayed fret of 7 is A4 (open + 2),
            // not D5 (open + 7). `saturating_sub` correctly returns 0
            // for fret <= offset, which keeps "open" as the pitch for
            // the physically impossible drone-fret-1..4 range too.
            let offset = fret_offsets.get(string_idx).copied().unwrap_or(0);
            let semitones = fret.saturating_sub(offset);
            let midi = MidiNote(open.0.saturating_add(semitones));
            format!("{:<2}", midi.pitch_class_name())
        }
        _ => "  ".to_string(),
    }
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
/// `pub` so the `calibration` module can reuse the same click sample
/// for round-trip latency measurement (different consumer, same signal).
pub fn metronome_click(sample_rate: u32) -> Vec<f32> {
    let n = (sample_rate as f32 * 0.05) as usize;
    let mut buf = sine(Frequency(1000.0), sample_rate, n);
    exp_decay(&mut buf, sample_rate, 0.012);
    for s in buf.iter_mut() {
        *s *= 0.3;
    }
    buf
}

/// Run a `pre_roll`-tick count-in before the main record/play loop starts.
/// Always audible — independent of any `--no-metronome` choice — because
/// the entire point is to give the user time to react. Aborts cleanly on
/// Ctrl-C / `q + Enter` so the user isn't stuck through 16 beats if they
/// changed their mind. No-op when `pre_roll == 0`.
fn run_pre_roll(
    pre_roll: u32,
    bpm: u32,
    output: &mut Option<OutputStream>,
    click: Option<&Vec<f32>>,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) -> Result<bool> {
    if pre_roll == 0 {
        return Ok(false); // not aborted
    }
    let beat_ms = (60_000 / bpm.max(1)) as u64;
    let beat_dur = std::time::Duration::from_millis(beat_ms);
    eprintln!("Pre-roll:   {pre_roll} count(s)…");
    for i in 1..=pre_roll {
        if twanga_tui::is_shutdown_requested() {
            return Ok(true);
        }
        if let Ok(input) = stdin_rx.try_recv()
            && is_quit_input(&input)
        {
            return Ok(true);
        }
        eprint!("  {i}/{pre_roll}");
        if let (Some(out), Some(click)) = (output.as_mut(), click) {
            out.write(click);
        }
        std::thread::sleep(beat_dur);
        eprint!("\r");
    }
    eprintln!("           "); // clear the in-place counter
    Ok(false)
}

/// In wait mode, block until the user plays a frequency that matches one of
/// the expected `(string, fret)` hits within [`WAIT_MATCH_CENTS`] of the
/// target. Polls Ctrl-C / `q` so the user can still abort.
fn wait_for_expected_note(
    input_state: &mut InputState,
    buf: &mut [f32],
    expected: &[(u8, u8)],
    tuning: &Tuning,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) -> Result<()> {
    let (stream, tuner) = input_state.as_mut().expect("wait mode needs input");

    // Drain whatever audio has accumulated in the cpal stream since the
    // last wait (or since the session began). Without this, the FIRST
    // wait sees seconds of pre-wait mic buffering — pre-roll noise,
    // mic-warmup transients, the user shuffling — and any of those
    // happening to YIN-match the expected pitch exits the wait
    // immediately. Symptom: "the first note always continues anyway
    // even if I don't play." Drain feeds the tuner so calibration
    // state advances, then `clear_for_wait` discards the YIN buffer,
    // queued readings, and any `onset_pending` flag — so only audio
    // that arrives AFTER this point can satisfy the match.
    loop {
        let n = stream.read(buf);
        if n == 0 {
            break;
        }
        tuner.feed(&buf[..n]);
    }
    tuner.clear_for_wait();

    loop {
        if twanga_tui::is_shutdown_requested() {
            return Ok(());
        }
        if let Ok(input) = stdin_rx.try_recv() {
            if is_quit_input(&input) {
                return Ok(());
            }
            if is_threshold_down(&input) {
                step_silence_threshold(tuner, false);
            } else if is_threshold_up(&input) {
                step_silence_threshold(tuner, true);
            }
        }
        let n = stream.read(buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            for r in tuner.take_readings() {
                if wait_reading_advances(&r, expected, tuning) {
                    return Ok(());
                }
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// Decide whether a single [`TunerReading`] should advance the
/// wait-mode cursor for the column whose expected hits are given.
/// Pure function — same `(reading, expected, tuning)` always gives
/// the same answer — so the wait-mode contract is unit-testable
/// without spinning up an `InputStream`.
///
/// Two conditions must hold:
///
/// 1. **Onset gate:** the reading came from a YIN window that
///    started at a fresh onset (`from_onset_window == true`). A
///    sustained tail of the previous note can still YIN-match the
///    next expected pitch as the window slides; without this gate
///    the cursor would advance on "still hearing the last note"
///    rather than "user actually played the next one." See
///    `twanga-dsp::onset` + docs/plans/onset-detection.md.
///
/// 2. **Pitch match:** the detected frequency is within
///    [`WAIT_MATCH_CENTS`] of any of the column's expected
///    `(string, fret)` placements on the current tuning.
fn wait_reading_advances(
    reading: &twanga_dsp::TunerReading,
    expected: &[(u8, u8)],
    tuning: &Tuning,
) -> bool {
    reading.from_onset_window && matches_any_expected(reading.detected, expected, tuning)
}

fn matches_any_expected(detected: Frequency, expected: &[(u8, u8)], tuning: &Tuning) -> bool {
    for (string_num, fret) in expected {
        let string_idx = (*string_num as usize).saturating_sub(1);
        let Some(s) = tuning.strings.get(string_idx) else {
            continue;
        };
        // semitones = max(0, fret - fret_offset). fret 0 always means
        // "open" (target_hz == open_hz). For offset strings like the
        // banjo drone (offset 5), fret 7 in the tab means open + 2
        // semitones, not open + 7 — match against A4, not D5.
        let semitones = fret.saturating_sub(s.fret_offset);
        let open_hz = s.open.to_frequency().hz();
        let target_hz = open_hz * 2_f32.powf(semitones as f32 / 12.0);
        let cents = 1200.0 * (detected.hz() / target_hz).log2();
        if cents.abs() < WAIT_MATCH_CENTS {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod wait_match_tests {
    //! Coverage for the wait-mode match logic. Lives in
    //! [`wait_reading_advances`] + [`matches_any_expected`]; both
    //! are pure functions deliberately extracted so the wait-mode
    //! contract can be tested without an `InputStream` / `Tuner`
    //! pipeline. Integration-flavoured tests at the
    //! [`twanga_dsp::Tuner`] layer cover the upstream onset gate.
    use super::*;
    use twanga_core::{Frequency, MidiNote};

    fn reading(detected_hz: f32, from_onset: bool) -> twanga_dsp::TunerReading {
        twanga_dsp::TunerReading {
            detected: Frequency(detected_hz),
            label: String::new(),
            target: Frequency(0.0),
            cents: 0.0,
            from_onset_window: from_onset,
        }
    }

    #[test]
    fn matches_any_expected_accepts_exact_pitch_on_open_string() {
        // Standard guitar string 1 (high E, MIDI 64) at fret 0 → match exact E4.
        let tuning = Tuning::standard_guitar();
        let e4 = MidiNote(64).to_frequency();
        assert!(matches_any_expected(e4, &[(1, 0)], &tuning));
    }

    #[test]
    fn matches_any_expected_accepts_fretted_pitch() {
        // Standard guitar string 2 (B3, MIDI 59) at fret 1 → C4 (MIDI 60).
        let tuning = Tuning::standard_guitar();
        let c4 = MidiNote(60).to_frequency();
        assert!(matches_any_expected(c4, &[(2, 1)], &tuning));
    }

    #[test]
    fn matches_any_expected_rejects_unrelated_pitch() {
        // A4 doesn't match an open E2 (string 6, fret 0).
        let tuning = Tuning::standard_guitar();
        let a4 = MidiNote(69).to_frequency();
        assert!(!matches_any_expected(a4, &[(6, 0)], &tuning));
    }

    #[test]
    fn matches_any_expected_accepts_within_cents_tolerance() {
        // 49 cents sharp of A4 (< WAIT_MATCH_CENTS = 50) — accept.
        let tuning = Tuning::standard_guitar();
        let a4 = MidiNote(69).to_frequency().hz();
        let near = Frequency(a4 * 2_f32.powf(49.0 / 1200.0));
        assert!(matches_any_expected(near, &[(1, 5)], &tuning));
    }

    #[test]
    fn matches_any_expected_rejects_outside_cents_tolerance() {
        // 51 cents sharp of A4 (> WAIT_MATCH_CENTS = 50) — reject.
        let tuning = Tuning::standard_guitar();
        let a4 = MidiNote(69).to_frequency().hz();
        let far = Frequency(a4 * 2_f32.powf(51.0 / 1200.0));
        assert!(!matches_any_expected(far, &[(1, 5)], &tuning));
    }

    #[test]
    fn matches_any_expected_accepts_any_of_multiple_hits() {
        // Chord column: hits on strings 1+3. Detected pitch matches
        // string 3 → accept (doesn't have to match all).
        let tuning = Tuning::standard_guitar();
        let g3 = MidiNote(55).to_frequency(); // standard guitar string 3 open
        assert!(matches_any_expected(g3, &[(1, 0), (3, 0)], &tuning));
    }

    #[test]
    fn matches_any_expected_honours_5string_banjo_fret_offset() {
        // The banjo drone (string 5) has fret_offset = 5 — fret 7 in
        // a tab means "5 frets above open + 2 more semitones",
        // i.e. open + 2 semitones in pitch. Match against A4 (open
        // + 2 = g4 → A4).
        let tuning = Tuning::standard_banjo();
        let a4 = MidiNote(69).to_frequency();
        assert!(matches_any_expected(a4, &[(5, 7)], &tuning));
    }

    #[test]
    fn wait_reading_advances_requires_both_onset_and_pitch_match() {
        let tuning = Tuning::standard_guitar();
        let e4 = MidiNote(64).to_frequency().hz();
        let expected = [(1, 0)];

        // Both true → advance.
        assert!(wait_reading_advances(
            &reading(e4, true),
            &expected,
            &tuning
        ));

        // Onset false, pitch match → DON'T advance (sustained tail).
        assert!(!wait_reading_advances(
            &reading(e4, false),
            &expected,
            &tuning
        ));

        // Onset true, pitch mismatch → DON'T advance (wrong note played).
        let a4 = MidiNote(69).to_frequency().hz();
        assert!(!wait_reading_advances(
            &reading(a4, true),
            &expected,
            &tuning
        ));

        // Both false → DON'T advance.
        assert!(!wait_reading_advances(
            &reading(a4, false),
            &expected,
            &tuning
        ));
    }

    #[test]
    fn wait_reading_advances_empty_expected_never_matches() {
        // Defensive: a column with no hits (rest) shouldn't somehow
        // be wait-mode'd into — the caller already gates on
        // `column.hits.is_empty()` but pin the inner contract too.
        let tuning = Tuning::standard_guitar();
        let e4 = MidiNote(64).to_frequency().hz();
        assert!(!wait_reading_advances(&reading(e4, true), &[], &tuning));
    }
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

#[cfg(test)]
mod docs_tests {
    use super::*;

    /// `include_str!` guarantees the files exist at compile time, but
    /// nothing stops a future hand-edit from accidentally truncating
    /// one to zero bytes. Pin "every page has content + a top-level H1".
    #[test]
    fn every_embedded_page_is_non_empty_with_h1() {
        for (slug, blurb, body) in DOCS_PAGES {
            assert!(!body.is_empty(), "embedded doc '{slug}' is empty");
            assert!(!blurb.is_empty(), "blurb for '{slug}' is empty");
            assert!(
                body.starts_with("# "),
                "embedded doc '{slug}' must start with an H1, got: {:?}",
                &body[..body.len().min(40)]
            );
        }
    }

    #[test]
    fn slugs_are_unique() {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (slug, _, _) in DOCS_PAGES {
            assert!(seen.insert(slug), "duplicate doc slug: {slug}");
        }
    }

    #[test]
    fn docs_page_text_returns_body_for_known_slug() {
        let body = docs_page_text("tuner").expect("tuner page lookup");
        assert!(body.contains("# Tuner"));
    }

    #[test]
    fn docs_page_text_is_case_insensitive() {
        // CLI users sometimes type `Tuner` or `TUNER`. Mirror the
        // behaviour `run_docs` exposes via `to_lowercase()`.
        assert!(docs_page_text("TUNER").is_ok());
        assert!(docs_page_text("Recorder").is_ok());
    }

    #[test]
    fn docs_page_text_errors_on_unknown_slug() {
        let err = docs_page_text("not-a-feature").expect_err("should error");
        let msg = err.to_string();
        assert!(msg.contains("not-a-feature"), "unexpected message: {msg}");
        assert!(
            msg.contains("twanga docs"),
            "should hint at the listing: {msg}"
        );
    }

    #[test]
    fn docs_listing_text_includes_every_slug() {
        let listing = docs_listing_text();
        for (slug, _, _) in DOCS_PAGES {
            assert!(
                listing.contains(slug),
                "listing missing slug '{slug}': {listing}"
            );
        }
    }

    /// The GUI's `DOCS_FEATURES` array in `frontend/web/app.html` mirrors
    /// the slugs here — they have to stay in sync or `twanga docs` and
    /// the web docs viewer drift apart. We can't import JS into the
    /// Rust test, but we can at least pin the expected slug set; any
    /// change here is a deliberate sync point with the JS side.
    #[test]
    fn slug_set_matches_expected_features() {
        let expected = [
            "tuner",
            "recorder",
            "playback",
            "patterns",
            "editor",
            "importer",
            "tunings",
            "calibrate",
            "hardware",
            "user-guide",
        ];
        let actual: Vec<&str> = DOCS_PAGES.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(
            actual, expected,
            "if you add/remove a feature, update both DOCS_PAGES \
             (Rust, here) and DOCS_FEATURES (JS, frontend/web/app.html) \
             and the bundle copy in `.github/workflows/pages.yml`",
        );
    }
}
