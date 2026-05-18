use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use twanga_audio::{InputStream, OutputStream};
use twanga_core::{Frequency, TunedString, Tuning};
use twanga_dsp::{Tuner, TunerMode, TunerReading};
use twanga_synth::{exp_decay, sine};
use twanga_tabs::{
    alphatex::{self, AlphaTexWriter, ParsedTab},
    TabEvent, TabRecorder,
};
use twanga_tui::{color, MultiLineDisplay, StatusLine};

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
        /// Tuning preset. If omitted, the CLI prompts interactively.
        #[arg(long)]
        tuning: Option<String>,
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
        /// playable range are silently dropped.
        #[arg(long)]
        tuning: Option<String>,
        /// Override the tempo from the file (BPM).
        #[arg(long)]
        bpm: Option<u32>,
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
    },
    /// Live tab recorder — capture played notes as horizontal ASCII tab notation.
    /// Any argument left unset triggers an interactive prompt (with the default
    /// pre-filled — just press enter to accept).
    Record {
        /// Tuning preset.
        #[arg(long)]
        tuning: Option<String>,
        /// Tempo for the time grid (BPM).
        #[arg(long)]
        bpm: Option<u32>,
        /// Note value per column. Accepts `1/4`, `1/8`, `1/16`, `1/32`.
        #[arg(long)]
        resolution: Option<String>,
        /// Number of columns per scrolling block.
        #[arg(long)]
        block_width: Option<usize>,
    },
    /// List available audio input devices.
    Devices,
    /// Convert a tab file from one format to another.
    Convert { input: String, output: String },
}

/// CLI mode menu for `tune`: index 0 is chromatic; the rest map to `Tuning::PRESETS`.
fn tune_menu_options() -> Vec<&'static str> {
    let mut v = Vec::with_capacity(1 + Tuning::PRESETS.len());
    v.push("(no instrument — chromatic tuner)");
    v.extend_from_slice(Tuning::PRESETS);
    v
}

fn resolve_mode(arg: Option<String>) -> Result<TunerMode> {
    if let Some(name) = arg {
        let tuning = Tuning::from_preset(&name).ok_or_else(|| {
            anyhow!(
                "unknown preset '{name}'. options: {}",
                Tuning::PRESETS.join(", ")
            )
        })?;
        return Ok(TunerMode::Strings(tuning));
    }

    let options = tune_menu_options();
    let idx = twanga_tui::select("Choose a tuning:", &options)?;
    if idx == 0 {
        Ok(TunerMode::Chromatic)
    } else {
        let preset = options[idx];
        let tuning = Tuning::from_preset(preset)
            .ok_or_else(|| anyhow!("preset registry desync; report this bug"))?;
        Ok(TunerMode::Strings(tuning))
    }
}

fn resolve_tuning(arg: Option<String>) -> Result<Tuning> {
    if let Some(name) = arg {
        return Tuning::from_preset(&name).ok_or_else(|| {
            anyhow!(
                "unknown preset '{name}'. options: {}",
                Tuning::PRESETS.join(", ")
            )
        });
    }
    let idx = twanga_tui::select("Choose a tuning to record against:", Tuning::PRESETS)?;
    Tuning::from_preset(Tuning::PRESETS[idx])
        .ok_or_else(|| anyhow!("preset registry desync; report this bug"))
}

/// If `arg` is provided, return as-is. Otherwise prompt with the file's own
/// tuning as the default first option (press enter to accept). Returns `None`
/// for "use the file's tuning unchanged" or `Some(preset)` for transposition.
///
/// Non-TTY callers skip the prompt and get `None`, matching the previous
/// behaviour where `play` without `--tuning` just used the file's tuning.
fn resolve_play_tuning(
    arg: Option<String>,
    tab: &ParsedTab,
) -> Result<Option<String>> {
    if arg.is_some() {
        return Ok(arg);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(None);
    }
    let as_recorded = format!(
        "(as recorded in file: {})",
        tab.tuning_names.join(" ")
    );
    let mut owned: Vec<String> = Vec::with_capacity(1 + Tuning::PRESETS.len());
    owned.push(as_recorded);
    owned.extend(Tuning::PRESETS.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let idx = twanga_tui::select_with_default(
        "Choose a tuning for playback:",
        &refs,
        0,
    )?;
    if idx == 0 {
        Ok(None)
    } else {
        Ok(Some(Tuning::PRESETS[idx - 1].to_string()))
    }
}

/// If `arg` is provided, validate it; otherwise prompt with the default.
fn resolve_bpm(arg: Option<u32>) -> Result<u32> {
    if let Some(b) = arg {
        validate_bpm(b)?;
        return Ok(b);
    }
    twanga_tui::prompt_parsed("Tempo (BPM)", DEFAULT_BPM, |s| {
        let n: u32 = s.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
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

/// If `arg` is provided, parse it; otherwise prompt from the fixed list.
fn resolve_resolution(arg: Option<String>) -> Result<u32> {
    if let Some(r) = arg {
        return parse_resolution(&r);
    }
    const LABELS: &[&str] = &["1/4", "1/8", "1/16", "1/32"];
    const DENOMS: &[u32] = &[4, 8, 16, 32];
    let default_idx = LABELS.iter().position(|l| *l == DEFAULT_RESOLUTION).unwrap_or(1);
    let idx = twanga_tui::select_with_default("Resolution:", LABELS, default_idx)?;
    Ok(DENOMS[idx])
}

/// If `arg` is provided, validate it; otherwise prompt with the default.
fn resolve_block_width(arg: Option<usize>) -> Result<usize> {
    if let Some(b) = arg {
        validate_block_width(b)?;
        return Ok(b);
    }
    twanga_tui::prompt_parsed(
        "Block width (columns per scrolling block)",
        DEFAULT_BLOCK_WIDTH,
        |s| {
            let n: usize = s.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
            validate_block_width(n).map_err(|e| e.to_string())?;
            Ok(n)
        },
    )
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
    let num = parts.next().ok_or_else(|| anyhow!("invalid resolution '{s}'"))?;
    let denom = parts
        .next()
        .ok_or_else(|| anyhow!("invalid resolution '{s}' (expected `1/N`)"))?;
    if parts.next().is_some() {
        return Err(anyhow!("invalid resolution '{s}' (expected `1/N`)"));
    }
    if num != "1" {
        return Err(anyhow!(
            "resolution must be of the form `1/N` (got '{s}')"
        ));
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

fn run_strings(
    mut tuner: Tuner,
    mut stream: InputStream,
    strings: Vec<TunedString>,
) -> Result<()> {
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

/// Open a timestamped alphaTex recording file for `twanga record`. Creates the
/// `recordings/` directory if it doesn't exist, writes the alphaTex header,
/// and returns the path + a streaming writer ready for per-column writes.
fn open_recording_file(
    tuning: &Tuning,
    bpm: u32,
    resolution_denom: u32,
) -> Result<(PathBuf, AlphaTexWriter<BufWriter<File>>)> {
    let dir = PathBuf::from(RECORDINGS_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create '{}' directory", dir.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("recording-{secs}.alphatex"));
    let file = File::create(&path)
        .with_context(|| format!("failed to create '{}'", path.display()))?;
    let writer = AlphaTexWriter::new(BufWriter::new(file), tuning, bpm, resolution_denom)
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

fn run_recorder(
    tuning: Tuning,
    bpm: u32,
    resolution_denom: u32,
    block_width: usize,
) -> Result<()> {
    let mut stream = InputStream::open()?;
    let sample_rate = stream.sample_rate;
    let ms_per_col = 240_000 / (bpm * resolution_denom);

    let tuning_name = tuning.name.clone();
    let string_count = tuning.strings.len();

    let mut recorder = TabRecorder::new(&tuning, sample_rate, ms_per_col, block_width);
    // Chromatic mode: gives us raw detected frequencies (silence-gated) without
    // imposing the tuner's ±7 semitone string-distance gate, which would clip
    // high-fret recording. We do our own fret-aware string match below.
    let mut tuner = Tuner::new(TunerMode::Chromatic, sample_rate);

    let (recording_path, mut recording_writer) =
        open_recording_file(&tuning, bpm, resolution_denom)?;

    eprintln!("Tuning:     {tuning_name} ({string_count} strings)");
    eprintln!("Device:     {}", stream.device_name);
    eprintln!("Audio:      {sample_rate} Hz");
    eprintln!(
        "Tempo:      {bpm} BPM, 1/{resolution_denom} notes ({ms_per_col} ms/col)",
    );
    eprintln!(
        "Block:      {block_width} cols ({} ms wide)",
        block_width as u32 * ms_per_col,
    );
    eprintln!("Saving to:  {}", recording_path.display());
    eprintln!();
    eprintln!("─────────────────────────────────────────────────");
    eprintln!("  Controls: type 'q' + Enter to stop  (or Ctrl-C)");
    eprintln!("─────────────────────────────────────────────────");
    eprintln!();

    let n_rows = recorder.string_count();
    let mut display: Option<MultiLineDisplay> = None;
    let mut buf = vec![0.0_f32; READ_CHUNK];
    let stdin_rx = twanga_tui::spawn_line_reader();

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
            tuner.feed(&buf[..n]);
            for r in tuner.take_readings() {
                if let Some(m) = tuning.match_to_fret(r.detected, MAX_FRET) {
                    recorder.record_hit(m.string_idx, m.fret);
                }
            }
            for event in recorder.advance(n) {
                let (rows, column_marks, is_block_complete) = match &event {
                    TabEvent::ColumnTick { rows, column_marks } => {
                        (rows, column_marks, false)
                    }
                    TabEvent::BlockComplete { rows, column_marks } => {
                        (rows, column_marks, true)
                    }
                };

                // Every committed column gets written to alphaTex.
                recording_writer.write_column(column_marks)?;

                // Display: refresh in place for ColumnTick, finalize for BlockComplete.
                let d = display.get_or_insert_with(|| MultiLineDisplay::new(n_rows));
                d.render(rows)?;
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
    match cli.command {
        Command::Tune { tuning } => {
            twanga_tui::motd::print_banner()?;
            let mode = resolve_mode(tuning)?;
            run_tuner(mode)?;
        }
        Command::Record {
            tuning,
            bpm,
            resolution,
            block_width,
        } => {
            twanga_tui::motd::print_banner()?;
            let t = resolve_tuning(tuning)?;
            let bpm = resolve_bpm(bpm)?;
            let denom = resolve_resolution(resolution)?;
            let bw = resolve_block_width(block_width)?;
            run_recorder(t, bpm, denom, bw)?;
        }
        Command::Play {
            path,
            tuning,
            bpm,
            no_metronome,
            wait,
            loop_spec,
        } => {
            twanga_tui::motd::print_banner()?;
            // Parse first so the tuning prompt can show what's in the file.
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read '{}'", path.display()))?;
            let parsed = alphatex::parse(&content)
                .map_err(|e| anyhow!("failed to parse alphaTex: {e}"))?;
            if parsed.columns.is_empty() {
                return Err(anyhow!("'{}' has no notes to play", path.display()));
            }
            let tuning = resolve_play_tuning(tuning, &parsed)?;
            run_playback(path, parsed, tuning, bpm, !no_metronome, wait, loop_spec)?;
        }
        Command::Devices => {
            for name in twanga_audio::list_input_devices()? {
                println!("{name}");
            }
        }
        Command::Convert { input, output } => {
            println!("convert: not yet implemented ({input} -> {output})");
        }
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

fn run_playback(
    path: PathBuf,
    parsed: ParsedTab,
    tuning_override: Option<String>,
    bpm_override: Option<u32>,
    metronome: bool,
    wait: bool,
    loop_spec: Option<String>,
) -> Result<()> {
    // Transpose if --tuning was provided (either explicitly via flag or via
    // the prompt). The transposed tab carries the target tuning's names in
    // its header, so downstream code (wait mode, display) sees one consistent
    // tuning throughout.
    let tab = if let Some(name) = tuning_override.as_deref() {
        let target = Tuning::from_preset(name).ok_or_else(|| {
            anyhow!(
                "unknown tuning preset '{name}'. options: {}",
                Tuning::PRESETS.join(", ")
            )
        })?;
        parsed.transpose_to(&target, MAX_FRET)
    } else {
        parsed
    };

    let (loop_start, loop_end, repeat) = parse_loop_spec(loop_spec.as_deref(), tab.columns.len())?;

    let bpm = bpm_override.unwrap_or(tab.tempo);
    let resolution_denom = tab.columns[0].duration_denom;
    let ms_per_col = 240_000 / (bpm * resolution_denom);
    let cols_per_beat = (resolution_denom as usize / 4).max(1);

    let tuning_for_wait: Option<Tuning> = if wait {
        Some(
            tab.tuning()
                .ok_or_else(|| anyhow!("'\\tuning' header is missing or unparseable"))?,
        )
    } else {
        None
    };

    let mut output = if metronome {
        Some(OutputStream::open()?)
    } else {
        None
    };
    let click = output.as_ref().map(|o| metronome_click(o.sample_rate));

    let (mut input_state, mut input_buf) = if wait {
        let s = InputStream::open()?;
        let sr = s.sample_rate;
        (Some((s, Tuner::new(TunerMode::Chromatic, sr))), vec![0.0_f32; READ_CHUNK])
    } else {
        (None, Vec::new())
    };

    let name_width = tab
        .tuning_names
        .iter()
        .map(|n| n.len())
        .max()
        .unwrap_or(0);

    eprintln!("Playback:   {}", path.display());
    if let Some(subtitle) = tab.subtitle.as_deref() {
        eprintln!("Subtitle:   {subtitle}");
    }
    if let Some(name) = tuning_override.as_deref() {
        eprintln!("Transposed: {name} ({})", tab.tuning_names.join(" "));
    } else {
        eprintln!("Tuning:     {} (from file)", tab.tuning_names.join(" "));
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
            let rows = render_playback_rows(&tab, col_idx, PLAYBACK_WINDOW_COLS, name_width);
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
fn parse_loop_spec(
    spec: Option<&str>,
    total: usize,
) -> Result<(usize, usize, bool)> {
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
                return Err(anyhow!(
                    "loop end ({end}) exceeds column count ({total})"
                ));
            }
            Ok((start, end, true))
        }
    }
}

/// Render the playback view: one row per string showing a window of columns
/// centred on `current_col`, with the current column bracketed. Last row is a
/// `[col / total]` progress line.
fn render_playback_rows(
    tab: &ParsedTab,
    current_col: usize,
    window_cols: usize,
    name_width: usize,
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
        "{pad}   col {}/{}  (bar {}, beat {})",
        current_col + 1,
        tab.columns.len(),
        current_col / tab.columns[0].duration_denom as usize + 1,
        (current_col % tab.columns[0].duration_denom as usize) / ((tab.columns[0].duration_denom / 4) as usize).max(1) + 1,
    ));

    rows
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
        let Some(s) = tuning.strings.get(string_idx) else { continue };
        let open_hz = s.open.to_frequency().hz();
        let target_hz = open_hz * 2_f32.powf(*fret as f32 / 12.0);
        let cents = 1200.0 * (detected.hz() / target_hz).log2();
        if cents.abs() < WAIT_MATCH_CENTS {
            return true;
        }
    }
    false
}

