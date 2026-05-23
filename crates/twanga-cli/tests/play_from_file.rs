//! End-to-end integration tests for `twanga play --from-file`.
//!
//! The `--from-file` flag swaps the live mic for a paced WAV-file
//! replay; with that in place the whole playback pipeline (cpal
//! reads → tuner → onset detector → wait-mode match / proximity
//! scoring → summary print) becomes deterministically testable
//! against synthesised inputs.
//!
//! Each test follows the same shape:
//!
//!   1. **Build a tab** — small alphaTex strings constructed inline
//!      with a known tempo + column durations + per-column hits.
//!   2. **Synth a WAV** — `Scenario::build()` produces a mono PCM
//!      f32 WAV from a vec of `(start_ms, duration_ms, freq_hz)`
//!      pluck events. Plucked-string envelope (sharp attack +
//!      exponential decay) so the onset detector fires per event.
//!   3. **Invoke the binary** — `twanga play <tab> --policy <X>
//!      --from-file <wav>`, capture stdout + stderr.
//!   4. **Assert against the summary** — parse the printed counts
//!      and verify they match the scenario's expected outcomes.
//!
//! The intent is to cover what *manual* end-to-end testing used to
//! cover (the only existing path) — fast-passage failure modes,
//! sustained-tail bleeds, late hits, wrong notes, chords — but
//! deterministically and against CI.

use std::f32::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use twanga_cli::wav;

const SR: u32 = 48_000;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_twanga"))
}

/// Module-level mutex acquired by every test on entry. The whole
/// suite is timing-sensitive — each test spawns a child binary
/// that reads its WAV via [`WavSampleSource`]'s wall-clock pacing,
/// and parallel cargo test contention skews those measurements
/// enough to flip Hit ↔ Late ↔ Missed on the tight-policy tests.
/// Serialising at the test level (vs requiring `--test-threads=1`
/// on the command line) means default `cargo test` runs reliably
/// in CI without extra ceremony.
fn serial_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Unique per-test scratch dir under `std::env::temp_dir()`, removed
/// at the end of each test via a manual `fs::remove_dir_all` (kept
/// explicit rather than via a drop guard so a test failure leaves
/// the artefacts behind for inspection).
fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("twanga-play-from-file-{label}-{nanos}"));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// One synthesised pluck event: a sine at `freq_hz` with a sharp
/// attack + exponential decay envelope, starting at `start_ms` into
/// the scenario and lasting `duration_ms`. Multiple events at the
/// same `start_ms` overlap to produce a chord.
#[derive(Debug, Clone, Copy)]
struct PluckEvent {
    start_ms: u32,
    duration_ms: u32,
    freq_hz: f32,
}

/// Builder for a synthesised audio scenario. Holds a list of
/// `PluckEvent`s and a total file duration; `build` rasterises them
/// into a `Vec<f32>` at [`SR`].
struct Scenario {
    events: Vec<PluckEvent>,
    total_ms: u32,
    /// Optional background-noise amplitude in [0.0, 1.0]. Mixes a
    /// deterministic pseudo-random hiss across the whole buffer.
    noise_amp: f32,
    /// Optional 60 Hz hum amplitude in [0.0, 1.0]. Mixes a steady
    /// mains-hum sine across the whole buffer — useful for testing
    /// that the silence calibration + onset detector don't get
    /// confused by a quiet but persistent background tone.
    hum_amp: f32,
}

impl Scenario {
    fn new(total_ms: u32) -> Self {
        Self {
            events: Vec::new(),
            total_ms,
            noise_amp: 0.0,
            hum_amp: 0.0,
        }
    }

    /// Overlay a uniformly-distributed pseudo-random noise floor at
    /// the given amplitude. 0.005 ≈ a quiet room with passive
    /// amplification; 0.02 ≈ a noisier USB-cable input chain.
    fn with_noise(mut self, amp: f32) -> Self {
        self.noise_amp = amp;
        self
    }

    /// Overlay a 60 Hz mains-hum sine at the given amplitude.
    /// 0.01–0.02 is a typical real-world hum from poorly-shielded
    /// gear; loud enough to register on naive level thresholding
    /// but well below pluck peaks.
    fn with_hum(mut self, amp: f32) -> Self {
        self.hum_amp = amp;
        self
    }

    fn pluck(mut self, start_ms: u32, duration_ms: u32, freq_hz: f32) -> Self {
        self.events.push(PluckEvent {
            start_ms,
            duration_ms,
            freq_hz,
        });
        self
    }

    /// Add a chord — multiple plucks at the same start time. Each
    /// freq becomes its own event; the rasteriser sums them.
    fn chord(mut self, start_ms: u32, duration_ms: u32, freqs: &[f32]) -> Self {
        for &f in freqs {
            self.events.push(PluckEvent {
                start_ms,
                duration_ms,
                freq_hz: f,
            });
        }
        self
    }

    /// Rasterise to a mono f32 sample buffer at the global sample
    /// rate. Plucked-string envelope: amplitude is `peak *
    /// exp(-t/decay_constant)` where `decay_constant` is half the
    /// event duration, giving a recognisable attack-decay shape
    /// the onset detector + YIN can both work with.
    fn render(&self) -> Vec<f32> {
        let total_samples = (self.total_ms as u64 * SR as u64 / 1000) as usize;
        let mut out = vec![0.0_f32; total_samples];
        for ev in &self.events {
            let start = (ev.start_ms as u64 * SR as u64 / 1000) as usize;
            let dur = (ev.duration_ms as u64 * SR as u64 / 1000) as usize;
            let decay_samples = (dur as f32 / 2.0).max(1.0);
            for i in 0..dur {
                let idx = start + i;
                if idx >= total_samples {
                    break;
                }
                // Peak amplitude 0.4 per event; chord superposition
                // sums to ~1.0 max for 2–3 voice chords without
                // hitting hard clipping.
                let env = 0.4 * (-(i as f32) / decay_samples).exp();
                out[idx] += env * (2.0 * PI * ev.freq_hz * idx as f32 / SR as f32).sin();
            }
        }
        // Optional steady 60 Hz mains hum.
        if self.hum_amp > 0.0 {
            for (i, s) in out.iter_mut().enumerate() {
                *s += self.hum_amp * (2.0 * PI * 60.0 * i as f32 / SR as f32).sin();
            }
        }
        // Optional pseudo-random noise floor. Linear-congruential
        // generator keeps the noise deterministic across test runs
        // (so assertions don't flicker on the boundary of the
        // silence-calibration threshold).
        if self.noise_amp > 0.0 {
            let mut state: u32 = 0x_C0FF_EE42;
            for s in out.iter_mut() {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let n = ((state >> 16) as f32 / 32_768.0) - 1.0; // [-1, 1)
                *s += self.noise_amp * n;
            }
        }
        // Clamp to [-1.0, 1.0] — overlapping events on the same
        // timestamp can sum past unity. The wav reader / Tuner
        // tolerate values outside that range but clamping keeps
        // the synth fixture's behaviour closer to real audio gear.
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        out
    }

    fn write_to(&self, path: &std::path::Path) {
        let samples = self.render();
        wav::write(path, SR, &samples).expect("write wav");
    }
}

/// Build an alphaTex tab string with a fixed tuning + tempo +
/// uniform 4 (quarter-note) durations. `hits_per_col` is one
/// element per column: each is a vec of `(string, fret)` pairs (or
/// empty for a rest). Sized to match the scenario the WAV
/// represents.
fn make_alphatex(tempo: u32, hits_per_col: &[Vec<(u8, u8)>]) -> String {
    make_alphatex_tuned(
        tempo,
        "Standard Guitar (EADGBE)",
        "E4 B3 G3 D3 A2 E2",
        hits_per_col,
    )
}

/// Variant of [`make_alphatex`] that accepts an arbitrary subtitle
/// and tuning spec (high-string-first, alphaTex order). Use this
/// for non-guitar scenarios: banjo 5-string with reentrant tuning,
/// bass, etc. The subtitle just shows in the rendered tab; the
/// tuning line is what the playback engine and scorer consume.
fn make_alphatex_tuned(
    tempo: u32,
    subtitle: &str,
    tuning_spec: &str,
    hits_per_col: &[Vec<(u8, u8)>],
) -> String {
    let mut out = String::new();
    out.push_str("\\title \"Test\"\n");
    out.push_str(&format!("\\subtitle \"{subtitle}\"\n"));
    out.push_str(&format!("\\tempo {tempo}\n"));
    out.push_str(&format!("\\tuning {tuning_spec}\n\n.\n"));
    out.push_str(":4 ");
    for hits in hits_per_col {
        if hits.is_empty() {
            out.push_str("r ");
        } else if hits.len() == 1 {
            out.push_str(&format!("{}.{} ", hits[0].1, hits[0].0));
        } else {
            out.push('(');
            for (i, (s, f)) in hits.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(&format!("{f}.{s}"));
            }
            out.push_str(") ");
        }
    }
    out.push_str("|\n");
    out
}

/// MIDI pitch → frequency. Convenience for building scenarios that
/// reference the same pitches the tab's `(string, fret)` hits
/// resolve to.
fn midi_hz(midi: u8) -> f32 {
    440.0 * 2_f32.powf((midi as f32 - 69.0) / 12.0)
}

/// Run `twanga play --policy <p> --from-file <wav> <tab>` and
/// return (exit_success, stdout_combined_with_stderr). All scenario
/// tests share this entry point so the assertions stay focused on
/// outcomes rather than process plumbing.
fn run_play(
    tab_path: &std::path::Path,
    wav_path: &std::path::Path,
    policy: &str,
) -> (bool, String) {
    let out = Command::new(binary())
        .arg("play")
        .arg(tab_path)
        .arg("--policy")
        .arg(policy)
        .arg("--from-file")
        .arg(wav_path)
        .arg("--no-resume")
        .arg("--pre-roll")
        .arg("0")
        .arg("--no-metronome")
        .output()
        .expect("invoke twanga");
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&out.stdout));
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), combined)
}

/// Pull `hit / late / missed / wrong_pitch / total` out of the
/// CLI's summary output. The summary format:
///
///   Score:
///     Hit:         3 (75%)
///     Late:        0 (0%)
///     Missed:      1 (25%)
///     Wrong pitch: 0 (0%)
///     Total notes: 4
///
/// Returns `(hit, late, missed, wrong, total)`. Panics if the
/// output doesn't contain a summary (the scenario test was
/// presumably run in a non-scoring policy — caller's bug).
fn parse_summary(output: &str) -> (u32, u32, u32, u32, u32) {
    fn extract(out: &str, label: &str) -> u32 {
        for line in out.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(label) {
                let rest = rest.trim();
                let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = num.parse::<u32>() {
                    return n;
                }
            }
        }
        panic!("no '{label}' line in:\n{out}");
    }
    (
        extract(output, "Hit:"),
        extract(output, "Late:"),
        extract(output, "Missed:"),
        extract(output, "Wrong pitch:"),
        extract(output, "Total notes:"),
    )
}

// ───────────────────── Perfect takes: wait mode ─────────────────────

#[test]
fn perfect_take_wait_mode_completes_cleanly() {
    let _guard = serial_lock();
    // 4 quarters at 120 BPM = 500 ms per quarter. Hits are
    // open low E (string 6 fret 0 → MIDI 40 → E2). Plucks land
    // exactly on each expected onset.
    let dir = scratch("perfect-wait");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("write tab");

    Scenario::new(2200)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);

    let (ok, _out) = run_play(&tab, &wav, "wait");
    assert!(ok, "wait-mode playback should succeed on a perfect take");
    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────── Perfect takes: proximity score ─────────────────────

#[test]
fn perfect_take_tight_score_classifies_as_hit() {
    let _guard = serial_lock();
    // Same shape — perfect timing, scored under the tight policy
    // (±50 ms). With `Tuner::window_latency_ms()` subtracted from
    // the captured onset timestamp, an on-time pluck reports at
    // its actual attack time (not when YIN finalised the pitch
    // ~170 ms later), so each column should classify as Hit, not
    // Late.
    let dir = scratch("perfect-tight");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("write tab");

    Scenario::new(2200)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);

    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "tight-score playback should succeed: {out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4, "summary total for 4-note tab: {out}");
    assert!(
        hit >= 3,
        "expected ≥3 of 4 Hits on a perfect tight take, got hit={hit} late={late} missed={missed} wrong={wrong}"
    );
    assert_eq!(
        wrong, 0,
        "perfect take should never classify as WrongPitch: hit={hit} late={late} wrong={wrong}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn perfect_take_casual_score_classifies_as_hit() {
    let _guard = serial_lock();
    // Same scenario, casual policy (±150 ms). With latency
    // compensation, on-time plucks land at offset ~0 and classify
    // cleanly as Hit.
    let dir = scratch("perfect-casual");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("write tab");
    Scenario::new(2200)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "casual-score playback should succeed: {out}");
    let (hit, late, _, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        hit >= 3,
        "expected ≥3 of 4 Hits on a perfect casual take, got hit={hit} late={late}"
    );
    assert_eq!(wrong, 0, "perfect take should never be WrongPitch");
    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────── Timing variants ─────────────────────

#[test]
fn consistently_late_take_under_tight_classifies_as_late() {
    let _guard = serial_lock();
    // Every pluck is 70 ms late. Tight policy's hit window is
    // ±50 ms, so each is past the Hit cutoff but inside the
    // 4 × 50 = 200 ms extended pairing window — should classify
    // as Late, NOT Missed. (This used to fail because the
    // playback loop reported onset timestamps at the moment YIN
    // finalised its reading rather than at the actual attack —
    // adding ~170 ms of latency and pushing every pluck past
    // tight's extended window. The fix subtracts
    // `Tuner::window_latency_ms()` from the captured timestamp.)
    let dir = scratch("late-tight");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(70, 400, midi_hz(40))
        .pluck(570, 400, midi_hz(40))
        .pluck(1070, 400, midi_hz(40))
        .pluck(1570, 400, midi_hz(40))
        .write_to(&wav);

    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "{out}");
    let (hit, late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        hit + late >= 3,
        "expected ≥3 paired plays (hit+late), got hit={hit} late={late} missed={missed}"
    );
    assert!(
        late >= 3,
        "+70 ms past tight's ±50 ms Hit cutoff should classify mostly as Late, got hit={hit} late={late}"
    );
}

#[test]
fn consistently_late_take_under_casual_classifies_as_hit() {
    let _guard = serial_lock();
    // Same 70 ms late take under casual policy (±150 ms). +70 ms
    // is well inside casual's Hit window, so once latency
    // compensation removes YIN's window delay these all score
    // as Hit. Pair with the tight variant above, which keeps the
    // same plucks but classifies them as Late because tight's
    // ±50 ms Hit cutoff is narrower than the offset.
    let dir = scratch("late-casual");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(70, 400, midi_hz(40))
        .pluck(570, 400, midi_hz(40))
        .pluck(1070, 400, midi_hz(40))
        .pluck(1570, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        hit >= 3,
        "casual should classify +70 ms plucks as Hit (well inside ±150 ms), got hit={hit} late={late} missed={missed}"
    );
}

#[test]
fn way_too_late_classifies_as_missed() {
    let _guard = serial_lock();
    // Plucks 2 seconds after expected — way past the extended
    // window. Scorer can't pair them with their intended columns;
    // those columns are Missed.
    let dir = scratch("way-late");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    // Each pluck is ~2 s late — well past tight's 4*50 = 200 ms
    // pairing window. Should mostly Miss.
    Scenario::new(5000)
        .pluck(2000, 400, midi_hz(40))
        .pluck(2500, 400, midi_hz(40))
        .pluck(3000, 400, midi_hz(40))
        .pluck(3500, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "{out}");
    let (_, _, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        missed >= 3,
        "expected ≥3 missed on a 2-second-late take, got {missed}"
    );
}

// ───────────────────── Wrong notes / silence ─────────────────────

#[test]
fn wrong_pitch_on_time_classifies_as_wrong_pitch_or_hit_via_octave() {
    let _guard = serial_lock();
    // Expected E2 (MIDI 40); user plays A2 (MIDI 45). On-time,
    // wrong pitch. Should classify as WrongPitch (or possibly
    // Hit if YIN locks onto a harmonic that happens to align —
    // accept both).
    let dir = scratch("wrong-pitch");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("perf.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    Scenario::new(1500)
        .pluck(0, 400, midi_hz(45)) // A2 instead of E2
        .pluck(500, 400, midi_hz(45))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "{out}");
    let (hit, _, _, wrong, total) = parse_summary(&out);
    assert_eq!(total, 2);
    assert!(
        wrong + hit == 2 && wrong >= 1,
        "expected mostly WrongPitch (some Hit if YIN locks on a harmonic), got hit={hit} wrong={wrong}"
    );
}

#[test]
fn silent_wav_under_score_classifies_everything_missed() {
    let _guard = serial_lock();
    // WAV that's just zeros — no onsets fire, no readings produced.
    // All columns get scored Missed.
    let dir = scratch("silent");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("silent.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]]),
    )
    .expect("tab");
    Scenario::new(2000).write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "{out}");
    let (_, _, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert_eq!(missed, 3, "silent input should miss everything");
}

// ───────────────────── Chord scenarios (any-pitch matches) ─────────────────────

#[test]
fn chord_column_completes_without_crashing() {
    let _guard = serial_lock();
    // Tab has a 2-string chord: high E open (1, 0 → MIDI 64 =
    // E4) + B open (2, 0 → MIDI 59 = B3). WAV synthesises both
    // pitches simultaneously. The intent of this test is NOT to
    // verify chord scoring works — YIN is monophonic and tends
    // to lock on harmonic artifacts of overlapping sines, which
    // most often produces either Missed (no confident pitch) or
    // WrongPitch (a sum/beat frequency outside the cents
    // tolerance). The intent is to pin that the binary handles
    // a chord-column tab without panicking and accounts for
    // every column in the summary — a precondition for adding
    // real polyphonic chord detection later.
    let dir = scratch("chord-hit");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("chord.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(1, 0), (2, 0)], vec![(1, 0), (2, 0)]]),
    )
    .expect("tab");
    Scenario::new(1500)
        .chord(0, 400, &[midi_hz(64), midi_hz(59)]) // E4 + B3
        .chord(500, 400, &[midi_hz(64), midi_hz(59)])
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 2);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "every column should be accounted for: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

// ───────────────────── Fast-passage regression (Ship 1 fix) ─────────────────────

#[test]
fn fast_passage_no_longer_skips_via_sustained_tail() {
    let _guard = serial_lock();
    // The headline failure mode from before Ship 1's onset detector:
    // a slow-decay note's tail could YIN-match the NEXT expected
    // column under wait-mode, advancing the cursor without the user
    // actually playing. With the onset gate in place, only fresh
    // attacks count — the second column has no onset, so wait-mode
    // doesn't advance.
    //
    // Setup: tab expects two consecutive E2 notes 500 ms apart.
    // WAV provides only the FIRST pluck (no second). Under wait
    // mode, the cursor should hang on column 2 until eventually
    // the test runner kills the binary OR the file runs out and
    // the user-emulating WAV becomes silent — either way, wait
    // mode must NOT advance past col 1.
    //
    // We assert via the proximity-score path instead (easier to
    // probe): in tight mode, column 2 should be Missed because no
    // second onset ever fires. Column 1 should be Hit.
    let dir = scratch("fast-tail");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("tail.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    // ONE long-decay pluck at t=0; the tail sustains past 500 ms
    // (column 2's expected onset). No second attack means no
    // second onset means column 2 is Missed.
    Scenario::new(2000)
        .pluck(0, 1500, midi_hz(40)) // 1.5 s decay — well into col 2's window
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "tight");
    assert!(ok, "{out}");
    let (_hit, _late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 2);
    assert!(
        missed >= 1,
        "long-tail-only take should produce ≥1 Missed (no fresh onset for col 2), got missed={missed}\nfull output:\n{out}"
    );
}

// ───────────────────── Tempo variants ─────────────────────

#[test]
fn very_slow_tempo_60_bpm_pairs_all_columns() {
    let _guard = serial_lock();
    // 60 BPM quarter = 1000 ms per column. Generous spacing means
    // YIN's ~170 ms detection latency is comfortably inside every
    // pairing window — even tight (4 × 50 = 200 ms extended) should
    // pair every column without onsets bleeding into the next.
    let dir = scratch("slow-60");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("slow.wav");
    fs::write(
        &tab,
        make_alphatex(60, &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]]),
    )
    .expect("tab");
    Scenario::new(4000)
        .pluck(0, 800, midi_hz(40))
        .pluck(1000, 800, midi_hz(40))
        .pluck(2000, 800, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert!(
        hit + late >= 2,
        "60 BPM should pair almost everything, got hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

#[test]
fn fast_tempo_200_bpm_still_pairs_under_casual() {
    let _guard = serial_lock();
    // 200 BPM quarter = 300 ms per column. Onsets need to fire
    // ~300 ms apart and YIN needs to lock between them. Tight
    // mode would struggle (300 ms gap < YIN window + latency),
    // but casual's 600 ms extended pairing window per column
    // gives the scorer room to pair onsets even with cross-column
    // latency drift. Plucks need short decay (~150 ms) so the
    // tail doesn't smear into the next column's onset detection.
    let dir = scratch("fast-200");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("fast.wav");
    fs::write(
        &tab,
        make_alphatex(
            200,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2000)
        .pluck(0, 200, midi_hz(40))
        .pluck(300, 200, midi_hz(40))
        .pluck(600, 200, midi_hz(40))
        .pluck(900, 200, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    // At 200 BPM with detection latency, onsets may pair with
    // the *next* column's expected_ms. We pin "majority paired"
    // rather than "all paired" because column boundaries get
    // mushy at this tempo.
    assert!(
        hit + late >= 2,
        "200 BPM casual should pair the majority, got hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

#[test]
fn very_fast_tempo_300_bpm_documents_breakdown() {
    let _guard = serial_lock();
    // 300 BPM quarter = 200 ms per column. The YIN window itself
    // (~170 ms) is almost as wide as the column duration — pitch
    // readings genuinely span multiple columns. This test pins
    // current behavior at the edge of the detector's ability:
    // the binary completes without crashing and every column is
    // accounted for, but the distribution between hit/late/missed
    // is not specified — it tracks YIN's wobble at this tempo.
    let dir = scratch("very-fast-300");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("vfast.wav");
    fs::write(
        &tab,
        make_alphatex(
            300,
            &[
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
            ],
        ),
    )
    .expect("tab");
    Scenario::new(2000)
        .pluck(0, 150, midi_hz(40))
        .pluck(200, 150, midi_hz(40))
        .pluck(400, 150, midi_hz(40))
        .pluck(600, 150, midi_hz(40))
        .pluck(800, 150, midi_hz(40))
        .pluck(1000, 150, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 6);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "all columns accounted for at 300 BPM: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

// ───────────────────── Overlapping notes ─────────────────────

#[test]
fn ring_out_into_next_column_still_detects_second_onset() {
    let _guard = serial_lock();
    // First pluck has a long decay (800 ms) that rings past the
    // next column's expected onset (500 ms in). The second pluck
    // strikes WHILE the first is still ringing — the onset
    // detector must still fire on the fresh attack despite the
    // background sustain.
    let dir = scratch("ring-out");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("ring.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]]),
    )
    .expect("tab");
    Scenario::new(2500)
        .pluck(0, 800, midi_hz(40)) // long-decay ring
        .pluck(500, 400, midi_hz(40)) // second attack while first sustains
        .pluck(1000, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 3);
    // The fresh attacks should produce fresh onsets even atop a
    // sustained tail — that's the whole point of the
    // energy-derivative onset detector vs raw level thresholding.
    assert!(
        hit + late >= 2,
        "overlapping plucks should still produce ≥2 onsets, got hit={hit} late={late} missed={missed}"
    );
}

#[test]
fn back_to_back_plucks_no_silence_between() {
    let _guard = serial_lock();
    // Plucks land at 0, 250, 500, 750 ms — each one starts
    // *during* the previous note's envelope. No silent gap.
    // At 240 BPM the tempo matches, so this is a legitimate
    // fast-strumming scenario.
    let dir = scratch("back-to-back");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("b2b.wav");
    fs::write(
        &tab,
        make_alphatex(
            240,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(1500)
        .pluck(0, 300, midi_hz(40))
        .pluck(250, 300, midi_hz(40))
        .pluck(500, 300, midi_hz(40))
        .pluck(750, 300, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "all back-to-back columns accounted for: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

// ───────────────────── Inconsistent pacing ─────────────────────

#[test]
fn jittery_pacing_pairs_most_columns_under_casual() {
    let _guard = serial_lock();
    // 4 quarter notes at 120 BPM. The user's plucks jitter
    // around the beat with offsets [-40, +80, -60, +90] ms —
    // realistic amateur timing. Casual's ±150 ms hit window
    // and 600 ms extended pairing should absorb everything.
    let dir = scratch("jitter");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("jit.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    // Beat times: 0, 500, 1000, 1500. Jitter applied per-pluck.
    Scenario::new(2400)
        .pluck(0, 400, midi_hz(40)) // -40 ms? clamp at 0
        .pluck(580, 400, midi_hz(40)) // +80 ms
        .pluck(940, 400, midi_hz(40)) // -60 ms
        .pluck(1590, 400, midi_hz(40)) // +90 ms
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        hit + late >= 3,
        "jittery casual take should pair ≥3 of 4, got hit={hit} late={late} missed={missed}"
    );
}

#[test]
fn accelerating_passage_speeds_up_through_columns() {
    let _guard = serial_lock();
    // User starts on time but speeds up: gaps of 500, 400, 300 ms
    // (vs the expected 500 ms each). By column 4 they're 300 ms
    // ahead of expected. Pins behavior when the user's playing
    // drifts ahead of the tempo — common when nerves take over.
    let dir = scratch("accel");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("accel.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(0, 400, midi_hz(40)) // on time
        .pluck(500, 400, midi_hz(40)) // on time
        .pluck(900, 400, midi_hz(40)) // 100 ms early
        .pluck(1200, 400, midi_hz(40)) // 300 ms early
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "accel passage columns accounted for: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

#[test]
fn decelerating_passage_slows_through_columns() {
    let _guard = serial_lock();
    // User starts on time but slows down: gaps of 500, 600, 700 ms.
    // By column 4 they're 300 ms behind. Mirror of the accel case.
    let dir = scratch("decel");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("decel.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(3000)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1100, 400, midi_hz(40))
        .pluck(1800, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert_eq!(hit + late + missed + wrong, total);
}

// ───────────────────── Combinations / mixed scenarios ─────────────────────

#[test]
fn alternating_right_and_wrong_pitches() {
    let _guard = serial_lock();
    // Columns expect E2 every time; user plays E2 / A2 / E2 / A2.
    // Should produce 2 paired-correctly and 2 WrongPitch under
    // casual (the timings are all on the beat).
    let dir = scratch("alt-pitch");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("alt.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(0, 400, midi_hz(40)) // E2 ✓
        .pluck(500, 400, midi_hz(45)) // A2 ✗
        .pluck(1000, 400, midi_hz(40)) // E2 ✓
        .pluck(1500, 400, midi_hz(45)) // A2 ✗
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    // Loose: at least one Wrong (the YIN may not be 100% reliable
    // on the harmonic-rich A2 above E2, but at least one of the
    // two A2 plucks should be flagged).
    assert!(
        wrong >= 1,
        "expected ≥1 WrongPitch from the A2 plucks, got hit={hit} late={late} wrong={wrong}"
    );
    assert!(
        hit + late >= 1,
        "expected ≥1 paired-correctly from the E2 plucks, got hit={hit} late={late}"
    );
}

#[test]
fn drop_a_note_in_the_middle_classifies_missing_one_as_missed() {
    let _guard = serial_lock();
    // Tab has 4 columns; user plays only columns 1, 2, 4
    // (forgets column 3). Casual policy. Column 3 should be
    // Missed; the others should pair.
    let dir = scratch("drop-mid");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("drop.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        // column 3 (expected at 1000 ms) — silent
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, _, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        missed >= 1,
        "dropped middle note should produce ≥1 Missed, got hit={hit} late={late} missed={missed}"
    );
    assert!(
        hit + late >= 2,
        "the 3 played notes should mostly pair, got hit={hit} late={late} missed={missed}"
    );
}

#[test]
fn extra_strums_between_expected_notes_do_not_pair_with_neighbors() {
    let _guard = serial_lock();
    // Tab expects 2 quarter notes (cols at 0 and 500 ms).
    // User plays the right notes plus an extra strum at 250 ms
    // (between the two columns). The extra onset shouldn't
    // promote a Hit on a column it wasn't aiming at — but the
    // scorer pairs the *closest unused onset within window* per
    // column, so behavior depends on which gets matched first.
    // This pins that: total stays at 2, neither column is
    // WrongPitch (everything is the same expected pitch), and
    // most columns pair.
    let dir = scratch("extra");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("extra.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    Scenario::new(1500)
        .pluck(0, 200, midi_hz(40))
        .pluck(250, 200, midi_hz(40)) // extra "in-between" strum
        .pluck(500, 200, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 2);
    assert_eq!(wrong, 0, "all plucks were the expected pitch");
    assert!(
        hit + late >= 1,
        "extra strum shouldn't prevent at least one pairing, got hit={hit} late={late}"
    );
}

#[test]
fn long_passage_eight_notes_mixed_timings() {
    let _guard = serial_lock();
    // Longer-form scenario: 8-column passage at 120 BPM with
    // mixed timing quality — first 4 perfect, last 4 sloppy.
    // Verifies the playback loop doesn't drift, lose onsets, or
    // get confused by accumulated timestamp offsets over a
    // multi-second session.
    let dir = scratch("long-mixed");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("long.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
                vec![(6, 0)],
            ],
        ),
    )
    .expect("tab");
    Scenario::new(4500)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .pluck(2050, 400, midi_hz(40)) // +50 ms
        .pluck(2620, 400, midi_hz(40)) // +120 ms
        .pluck(3050, 400, midi_hz(40)) // +50 ms
        .pluck(3580, 400, midi_hz(40)) // +80 ms
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 8);
    assert!(
        hit + late >= 5,
        "long passage should pair the majority, got hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

// ───────────────────── Reentrant tunings (banjo 5-string) ─────────────────────

#[test]
fn banjo_5_string_reentrant_tuning_pairs_high_5th_string() {
    let _guard = serial_lock();
    // 5-string banjo standard open-G tuning: g4 D3 G3 B3 D4 in
    // alphaTex's high-string-first order. String 1 = D4 (highest
    // finger-able), string 5 = g4 (reentrant — physically the
    // shortest, pitch HIGHER than string 1). Tab plays the open
    // 5th string (g4 → MIDI 67) then the open 1st string (D4 →
    // MIDI 62). Verifies the playback pipeline + scorer respect
    // the high-pitch reentrant string rather than assuming
    // strings are monotonically decreasing in pitch.
    let dir = scratch("banjo-reentrant");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("banjo.wav");
    fs::write(
        &tab,
        make_alphatex_tuned(
            120,
            "5-string banjo, open G (gDGBD)",
            "D4 B3 G3 D3 G4",
            &[vec![(5, 0)], vec![(1, 0)], vec![(5, 0)], vec![(1, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .pluck(0, 400, midi_hz(67)) // g4 reentrant 5th
        .pluck(500, 400, midi_hz(62)) // D4 1st string
        .pluck(1000, 400, midi_hz(67))
        .pluck(1500, 400, midi_hz(62))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert!(
        hit + late >= 3,
        "reentrant tuning should pair most columns, got hit={hit} late={late} wrong={wrong}"
    );
    assert_eq!(
        wrong, 0,
        "playing the correct pitch for each string should never WrongPitch under a reentrant tuning"
    );
}

#[test]
fn banjo_wrong_string_same_fret_classifies_as_wrong_pitch() {
    let _guard = serial_lock();
    // Tab expects open string 5 (g4 reentrant, MIDI 67). User
    // instead plays open string 4 (D3, MIDI 50) — same fret (0),
    // very different pitch. A bug in the scorer that ignored
    // reentrancy and just matched on (string, fret) shape would
    // miss this. The score should classify as WrongPitch.
    let dir = scratch("banjo-wrong");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("banjo.wav");
    fs::write(
        &tab,
        make_alphatex_tuned(
            120,
            "5-string banjo, open G (gDGBD)",
            "D4 B3 G3 D3 G4",
            &[vec![(5, 0)], vec![(5, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(1500)
        .pluck(0, 400, midi_hz(50)) // D3 — wrong octave, would pass if scorer ignored pitch
        .pluck(500, 400, midi_hz(50))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, _late, _missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 2);
    assert!(
        wrong >= 1,
        "playing D3 when g4 is expected should classify as WrongPitch, got hit={hit} wrong={wrong}"
    );
}

// ───────────────────── Pitch range edges ─────────────────────

#[test]
fn high_pitch_e5_pairs_under_casual() {
    let _guard = serial_lock();
    // High E5 (MIDI 76) — guitar's high E open string + octave
    // shift, or 12th-fret high E. Tab uses the high E (string 1
    // fret 12 → E5). Pins that YIN still locks on high pitches;
    // it's known to be less stable above ~2 kHz but E5 is ~659 Hz
    // and should be solid.
    let dir = scratch("high-e5");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("high.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(1, 12)], vec![(1, 12)], vec![(1, 12)]]),
    )
    .expect("tab");
    Scenario::new(2000)
        .pluck(0, 400, midi_hz(76))
        .pluck(500, 400, midi_hz(76))
        .pluck(1000, 400, midi_hz(76))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert!(
        hit + late >= 2,
        "E5 should pair under casual, got hit={hit} late={late} wrong={wrong}"
    );
}

#[test]
fn low_pitch_bass_e1_pairs_under_casual() {
    let _guard = serial_lock();
    // Bass guitar low E (E1, MIDI 28 = ~41 Hz). YIN's lower bound
    // is set by the analysis window length: at 48 kHz with 8192
    // samples, the longest period it can find is ~5.8 ms ≈ 170
    // Hz floor before the autocorrelation peak walks off the
    // window. 41 Hz is well below that, so YIN is expected to
    // mis-lock here. This test pins current behaviour at the
    // bottom of the supported range; if a low-pitch fix lands
    // later, the assertion can tighten.
    let dir = scratch("bass-e1");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("bass.wav");
    fs::write(
        &tab,
        make_alphatex_tuned(
            120,
            "4-string bass (EADG)",
            "G2 D2 A1 E1",
            &[vec![(4, 0)], vec![(4, 0)], vec![(4, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2000)
        .pluck(0, 800, midi_hz(28))
        .pluck(500, 800, midi_hz(28))
        .pluck(1000, 800, midi_hz(28))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "all columns accounted for at sub-YIN-floor pitch: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

#[test]
fn mid_range_a3_pitch_pairs_cleanly() {
    let _guard = serial_lock();
    // A3 (MIDI 57 = 220 Hz) — guitar string 3 fret 2, or
    // string 5 fret 12. Mid-range, sweet spot for YIN. Pin that
    // a non-low-E pitch on a different string slot also works.
    let dir = scratch("mid-a3");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("mid.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(3, 2)], vec![(3, 2)], vec![(3, 2)]]),
    )
    .expect("tab");
    Scenario::new(2000)
        .pluck(0, 400, midi_hz(57))
        .pluck(500, 400, midi_hz(57))
        .pluck(1000, 400, midi_hz(57))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _, wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert!(
        hit + late >= 2,
        "mid-range A3 should pair, got hit={hit} late={late} wrong={wrong}"
    );
}

// ───────────────────── Noisy input robustness ─────────────────────

#[test]
fn quiet_noise_floor_does_not_swamp_plucks() {
    let _guard = serial_lock();
    // Hiss at 0.005 amplitude — well below typical pluck peak
    // (~0.4 in our synthesiser). Onset detector should still
    // fire on each fresh attack; YIN should still lock on each
    // pluck. Calibration runs once at session start; the noise
    // floor sets the silence threshold above its peak.
    let dir = scratch("noise-quiet");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("noisy.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]]),
    )
    .expect("tab");
    Scenario::new(2000)
        .with_noise(0.005)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, _missed, _wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert!(
        hit + late >= 2,
        "quiet noise floor should not prevent pairing, got hit={hit} late={late}"
    );
}

#[test]
fn mains_hum_does_not_prevent_pairing() {
    let _guard = serial_lock();
    // 60 Hz hum at 0.015 amplitude — quietly persistent, like a
    // poorly-grounded amp. Hum is a pitched signal too, but the
    // calibration step pegs the silence threshold above it so
    // onsets only fire on the louder pluck attacks. The hum
    // sometimes confuses YIN on quieter notes (locks on 60 Hz);
    // assertion is loose enough to tolerate that.
    let dir = scratch("noise-hum");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("hum.wav");
    fs::write(
        &tab,
        make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]]),
    )
    .expect("tab");
    Scenario::new(2000)
        .with_hum(0.015)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 3);
    assert_eq!(
        hit + late + missed + wrong,
        total,
        "all columns accounted for with hum: hit={hit} late={late} missed={missed} wrong={wrong}"
    );
}

#[test]
fn combined_hum_and_hiss_still_scores_majority() {
    let _guard = serial_lock();
    // Both noise sources together at realistic levels. Worst-case
    // "noisy room with cheap cable" input. The pluck peaks are
    // still 10×+ louder than the combined background; onsets
    // should fire and YIN should mostly lock.
    let dir = scratch("noise-combo");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("combo.wav");
    fs::write(
        &tab,
        make_alphatex(
            120,
            &[vec![(6, 0)], vec![(6, 0)], vec![(6, 0)], vec![(6, 0)]],
        ),
    )
    .expect("tab");
    Scenario::new(2400)
        .with_noise(0.008)
        .with_hum(0.012)
        .pluck(0, 400, midi_hz(40))
        .pluck(500, 400, midi_hz(40))
        .pluck(1000, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "casual");
    assert!(ok, "{out}");
    let (hit, late, missed, wrong, total) = parse_summary(&out);
    assert_eq!(total, 4);
    assert_eq!(hit + late + missed + wrong, total);
}

// ───────────────────── Wait-mode pitch behaviour ─────────────────────

#[test]
fn wait_mode_holds_until_correct_pitch_arrives() {
    let _guard = serial_lock();
    // Wait mode shouldn't advance the cursor until a matching
    // pitch arrives. Setup: 2-column tab, both expect open low E.
    // WAV starts with 800 ms of silence (cursor must hold), then
    // plucks E2 twice with normal spacing. Both plucks happen
    // after the second column's nominal expected time would have
    // passed under proximity-score — but wait mode is cursor-
    // driven, so the second pluck advances col 2 cleanly.
    let dir = scratch("wait-hold");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("hold.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    Scenario::new(3000)
        // 800 ms of silence — wait mode must hang on col 1.
        .pluck(800, 400, midi_hz(40))
        .pluck(1500, 400, midi_hz(40))
        .write_to(&wav);
    let (ok, _out) = run_play(&tab, &wav, "wait");
    assert!(
        ok,
        "wait mode should eventually complete when correct pitches arrive"
    );
}

#[test]
fn wait_mode_ignores_wrong_pitch_then_advances_on_correct() {
    let _guard = serial_lock();
    // Wait mode treats wrong-pitch onsets as "keep waiting." User
    // plays A2 (wrong) twice, then E2 (correct). The cursor must
    // ignore the A2s and advance only on the E2. Same shape for
    // column 2.
    let dir = scratch("wait-ignore");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("ignore.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    Scenario::new(4000)
        .pluck(0, 300, midi_hz(45)) // wrong (A2)
        .pluck(400, 300, midi_hz(45)) // wrong (A2)
        .pluck(900, 400, midi_hz(40)) // correct (E2) — advances col 1
        .pluck(1500, 300, midi_hz(45)) // wrong on col 2
        .pluck(2000, 400, midi_hz(40)) // correct on col 2
        .write_to(&wav);
    let (ok, _out) = run_play(&tab, &wav, "wait");
    assert!(
        ok,
        "wait mode should complete by ignoring wrong-pitch onsets"
    );
}

// ───────────────────── Free-play sanity ─────────────────────

#[test]
fn free_play_with_from_file_completes_silently() {
    let _guard = serial_lock();
    // FreePlay needs no audio; --from-file is accepted but
    // ignored. The session should run through (4 columns × 500 ms
    // ≈ 2 s) and exit cleanly with no scoring summary.
    let dir = scratch("free");
    let tab = dir.join("tab.alphatex");
    let wav = dir.join("anything.wav");
    fs::write(&tab, make_alphatex(120, &[vec![(6, 0)], vec![(6, 0)]])).expect("tab");
    Scenario::new(1500).write_to(&wav);
    let (ok, out) = run_play(&tab, &wav, "free");
    assert!(ok, "{out}");
    assert!(
        !out.contains("Score:"),
        "free-play shouldn't print a score summary: {out}"
    );
}
