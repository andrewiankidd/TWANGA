//! WASM bindings layer for TWANGA's browser frontend.
//!
//! Thin re-exports of `twanga-core` / `twanga-dsp` / `twanga-tabs` shaped for
//! JavaScript callers via `wasm-bindgen`. No audio I/O here — that lives in
//! `twanga-audio`'s WebAudio backend (still TBD) and the JS-side glue.
//!
//! Public surface is deliberately small for now: the goal is to verify the
//! Rust → WASM → JS pipeline works end-to-end before piping real audio through it.

use wasm_bindgen::prelude::*;

/// Pick a random splash line from the shared `twanga_core::SPLASHES` list.
/// Browser-visible proof-of-life that the WASM bridge is working at all.
#[wasm_bindgen]
pub fn pick_splash(seed: u32) -> String {
    let splashes: Vec<&str> = twanga_core::splashes().collect();
    if splashes.is_empty() {
        return String::new();
    }
    splashes[seed as usize % splashes.len()].to_string()
}

/// Names of every built-in tuning preset slug, JSON-ready as a `string[]`.
/// The frontend's tuning picker will eventually consume this directly.
#[wasm_bindgen]
pub fn builtin_tuning_slugs() -> Vec<String> {
    twanga_core::Tuning::builtin_slugs()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

/// Parse a note name like `"A4"` / `"C#3"` into a MIDI number (or `None` if
/// invalid). Useful for the frontend to validate user-entered string pitches
/// when they're defining custom tunings, without re-implementing the parser.
#[wasm_bindgen]
pub fn midi_from_name(name: &str) -> Option<u8> {
    twanga_core::MidiNote::from_name(name).map(|n| n.0)
}

/// Inverse of [`midi_from_name`]. `0..=127` → name string.
#[wasm_bindgen]
pub fn midi_to_name(midi: u8) -> String {
    twanga_core::MidiNote(midi).name()
}

/// Human-readable display name for a built-in preset slug — e.g.
/// `"Standard Ukulele (Reentrant GCEA)"` for `"standard-ukulele"`. Returns
/// `None` for unknown slugs (matches `MidiNote::from_name`'s shape). Used
/// by the tuning picker so buttons read "Standard Ukulele" rather than
/// the developer-facing `standard-ukulele` slug.
#[wasm_bindgen]
pub fn preset_display_name(slug: &str) -> Option<String> {
    twanga_core::Tuning::from_preset(slug).map(|t| t.name)
}

/// Validate a `PresetEntry`-shaped JS object before persisting it. The browser
/// frontend builds custom tunings into the same `PresetEntry` shape the CLI
/// hand-edits in TOML — slug + display name + array of `{name, midi}` strings.
/// This function performs the same checks the CLI runs at add-time:
///
/// - Slug is non-empty, lowercase ASCII + digits + hyphens, no leading/trailing `-`.
/// - Slug doesn't collide with a built-in preset.
/// - Display name is non-empty.
/// - At least one string is defined.
/// - Every string's `name` is non-empty and `midi` is in `0..=127`.
///
/// Returns `Ok(())` on success, or a human-readable `String` error on failure
/// (same shape as the CLI's prompt validators). The frontend uses this to
/// surface red text under the offending form field rather than waiting until
/// the tuner constructor fails.
#[wasm_bindgen]
pub fn validate_preset_entry(preset_json: JsValue) -> Result<(), String> {
    let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
        .map_err(|e| format!("malformed tuning shape: {e}"))?;
    validate_entry(&entry)
}

/// JSON-friendly version of `twanga_core::FretMatch` for crossing the wasm
/// boundary. The `cents_off` field is dropped from the JS surface for now —
/// the Recorder only needs the chosen `(string_idx, fret)` pair.
#[derive(serde::Serialize)]
struct FretMatchJs {
    string_idx: usize,
    fret: u8,
}

/// Match a detected pitch to a `(string, fret)` pair against the given
/// `PresetEntry` + capo spec, using the exact same algorithm the CLI's
/// recorder uses (`Tuning::match_to_fret` after `Capo::apply`). Returns
/// `null` when no string can reach the pitch within `max_fret`, mirroring
/// the CLI's silent-discard behaviour.
///
/// The JS-side Recorder calls this for every detected frequency from the
/// chromatic-mode `WebTuner` and feeds the result into its column-tracking
/// state. Keeping the matching algorithm in Rust means the browser-recorded
/// `.alphatex` matches what the CLI would have written from the same audio.
#[wasm_bindgen]
pub fn match_pitch_to_fret(
    preset_json: JsValue,
    capo_spec: &str,
    freq_hz: f32,
    max_fret: u8,
) -> Result<JsValue, String> {
    let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
        .map_err(|e| format!("malformed tuning shape: {e}"))?;
    let tuning = entry.to_tuning();
    let capo = twanga_core::Capo::parse(capo_spec, tuning.strings.len())?;
    let effective = capo.apply(&tuning)?;
    match effective.match_to_fret(twanga_core::Frequency(freq_hz), max_fret) {
        Some(m) => Ok(serde_wasm_bindgen::to_value(&FretMatchJs {
            string_idx: m.string_idx,
            fret: m.fret,
        })
        .unwrap()),
        None => Ok(JsValue::NULL),
    }
}

/// Get the built-in `PresetEntry` JSON for a slug. Lets the JS Recorder
/// pass the same `PresetEntry`-shaped value into [`match_pitch_to_fret`]
/// and [`serialize_recording`] regardless of whether the active tuning is
/// built-in or user-defined — both code paths share one `JsValue` shape.
/// Returns `null` for unknown slugs.
#[wasm_bindgen]
pub fn builtin_preset_entry(slug: &str) -> JsValue {
    match twanga_core::Tuning::builtin_presets()
        .iter()
        .find(|p| p.slug == slug)
    {
        Some(p) => serde_wasm_bindgen::to_value(p).unwrap(),
        None => JsValue::NULL,
    }
}

/// Serialise a recording to alphaTex via the canonical `AlphaTexWriter`.
/// The CLI's `twanga record` writes through the exact same writer, so a
/// browser-saved file is bit-identical to a CLI-saved one given the same
/// inputs. Inputs:
///
/// - `preset_json`: `PresetEntry`-shaped JS object (built-in slugs come
///   from [`builtin_preset_entry`]; user tunings already live in this
///   shape in `localStorage`).
/// - `capo_spec`: `"3"` / `"0,2,2,2,2,2"` / `""` — same syntax as
///   `--capo` on the CLI.
/// - `bpm`, `resolution_denom`: same semantics as `--bpm` / `--resolution`.
/// - `columns_json`: `Array<Array<number | null>>`. Outer = columns in
///   time order, inner = per-string fret values (or `null` for "not
///   played that column"). Matches the column-grid score model the
///   renderers consume.
///
/// Returns the full alphaTex file content as a string, ready to write to
/// a blob URL for download (browser has no filesystem; the CLI writes to
/// `$CONFIG/twanga/recordings/`).
#[wasm_bindgen]
pub fn serialize_recording(
    preset_json: JsValue,
    capo_spec: &str,
    bpm: u32,
    resolution_denom: u32,
    columns_json: JsValue,
    title: Option<String>,
) -> Result<String, String> {
    let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
        .map_err(|e| format!("malformed tuning shape: {e}"))?;
    let tuning = entry.to_tuning();
    let capo = twanga_core::Capo::parse(capo_spec, tuning.strings.len())?;
    let columns: Vec<Vec<Option<u8>>> = serde_wasm_bindgen::from_value(columns_json)
        .map_err(|e| format!("malformed columns shape: {e}"))?;

    let title_ref = title.as_deref();
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = twanga_tabs::alphatex::AlphaTexWriter::new(
            &mut buf,
            &tuning,
            &capo,
            bpm,
            resolution_denom,
            title_ref,
        )
        .map_err(|e| e.to_string())?;
        for col in &columns {
            w.write_column(col).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())?;
    }
    String::from_utf8(buf).map_err(|e| e.to_string())
}

/// Stateful wrapper around `twanga_tabs::alphatex::ParsedTab`. Same shape
/// as `WebTuner` — JS holds an opaque handle, calls accessor methods to
/// read fields, and frees it via `.free()` when done. Used by the browser
/// Playback screen to ingest `.alphatex` files (drop-zone uploads + the
/// bundled examples) and ask for transpositions onto different tunings.
///
/// We don't expose `ParsedTab` as a serializable JSON blob because the
/// score model + transpose logic both want identity (you parse once, then
/// transpose multiple times to compare different target tunings). Same
/// reason `WebTuner` stays stateful.
#[wasm_bindgen]
pub struct WebParsedTab {
    inner: twanga_tabs::alphatex::ParsedTab,
}

#[derive(serde::Serialize)]
struct ColumnJs {
    duration_denom: u32,
    /// `[string_number_1_based, fret]` pairs. Empty = rest.
    hits: Vec<(u8, u8)>,
}

#[derive(serde::Serialize)]
struct DroppedNoteJs {
    column_index: usize,
    note: String,
}

/// Parse `.alphatex` text into a `WebParsedTab`. Returns the parse error
/// as a string on failure — matches the shape `serialize_recording` uses.
#[wasm_bindgen]
pub fn parse_alphatex(text: &str) -> Result<WebParsedTab, String> {
    twanga_tabs::alphatex::parse(text)
        .map(|inner| WebParsedTab { inner })
        .map_err(|e| e.to_string())
}

#[wasm_bindgen]
impl WebParsedTab {
    /// User-given title (from `\title "..."`). `None` for older files
    /// that predate the title feature.
    pub fn title(&self) -> Option<String> {
        self.inner.title.clone()
    }

    /// Subtitle stripped of any `; capo=...` machine annotation —
    /// human-readable label only. `None` for files without `\subtitle`.
    pub fn subtitle_display(&self) -> Option<String> {
        self.inner.subtitle_display()
    }

    /// File tempo (BPM). Defaults to 120 for files that omit `\tempo`.
    pub fn tempo(&self) -> u32 {
        self.inner.tempo
    }

    /// Open-string note names in string-number order, e.g. `["A4", "E4",
    /// "C4", "G4"]`. Used by the Playback screen's tuning header.
    pub fn tuning_names(&self) -> Vec<String> {
        self.inner.tuning_names.clone()
    }

    /// `Capo` spec from the `\subtitle` field (`; capo=<spec>` suffix),
    /// serialized back to the same `"3"` / `"0,2,2,2,2,2"` string format
    /// the CLI's `--capo` accepts. Empty string if the file has no capo.
    pub fn capo_spec(&self) -> String {
        self.inner.capo().map(|c| c.serialize()).unwrap_or_default()
    }

    /// Number of columns in the parsed tab. The Playback engine walks
    /// `0..columns_count()` calling `column_at(i)` for each.
    pub fn columns_count(&self) -> usize {
        self.inner.columns.len()
    }

    /// One column by index. Returns `{ duration_denom, hits }` where
    /// `hits` is `[[string_1_based, fret], ...]`. Empty hits = rest.
    /// Returns `null` if `idx >= columns_count()` rather than panicking,
    /// so the JS loop can defensively detect end-of-tab.
    pub fn column_at(&self, idx: usize) -> JsValue {
        match self.inner.columns.get(idx) {
            Some(col) => serde_wasm_bindgen::to_value(&ColumnJs {
                duration_denom: col.duration_denom,
                hits: col.hits.clone(),
            })
            .unwrap(),
            None => JsValue::NULL,
        }
    }

    /// Transpose to a different tuning, given a `PresetEntry`-shaped JS
    /// object (use `builtin_preset_entry(slug)` to get one for a built-in
    /// or pass a user-defined tuning directly). Returns the transposed
    /// tab as a fresh `WebParsedTab` plus the list of notes that couldn't
    /// fit on the target within `max_fret`. The browser Playback screen
    /// shows the dropped-notes list as a pre-flight "Skipped:" preamble
    /// before the cursor starts.
    pub fn transpose_to(&self, preset_json: JsValue, max_fret: u8) -> Result<WebParsedTab, String> {
        let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
            .map_err(|e| format!("malformed tuning shape: {e}"))?;
        let target = entry.to_tuning();
        let (transposed, _dropped) = self.inner.transpose_to_with_report(&target, max_fret);
        Ok(WebParsedTab { inner: transposed })
    }

    /// Same as `transpose_to` but also returns the list of dropped notes.
    /// Two methods because wasm-bindgen can't return both a heap-managed
    /// struct (`WebParsedTab`) and a serializable value in one tuple
    /// without going through JsValue serialisation for the whole thing —
    /// which costs the struct's identity. JS callers wanting both call
    /// `transpose_to_dropped_notes` first (cheap; no transposition done
    /// again under the hood — but for v1 we re-run since it's still O(N)
    /// over column hits and not hot).
    pub fn transpose_to_dropped_notes(
        &self,
        preset_json: JsValue,
        max_fret: u8,
    ) -> Result<JsValue, String> {
        let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
            .map_err(|e| format!("malformed tuning shape: {e}"))?;
        let target = entry.to_tuning();
        let (_transposed, dropped) = self.inner.transpose_to_with_report(&target, max_fret);
        let js_dropped: Vec<DroppedNoteJs> = dropped
            .into_iter()
            .map(|d| DroppedNoteJs {
                column_index: d.column_index,
                note: d.note,
            })
            .collect();
        Ok(serde_wasm_bindgen::to_value(&js_dropped).unwrap())
    }
}

/// Slug rules shared between [`validate_preset_entry`] and the tuner
/// constructors. Matches `validate_slug` in `twanga-cli`'s prompts so the
/// CLI and browser refuse the same inputs for the same reasons.
fn validate_slug(s: &str) -> Result<(), String> {
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

/// Full validation of a `PresetEntry`. Factored out so both the validate
/// helper and the custom-tuning constructor can reuse it without going
/// through `serde_wasm_bindgen` twice.
fn validate_entry(entry: &twanga_core::PresetEntry) -> Result<(), String> {
    validate_slug(&entry.slug)?;
    if twanga_core::Tuning::builtin_slugs().contains(&entry.slug.as_str()) {
        return Err(format!(
            "'{}' is a built-in preset slug — pick a different slug for your custom tuning",
            entry.slug
        ));
    }
    if entry.name.trim().is_empty() {
        return Err("display name cannot be empty".into());
    }
    if entry.strings.is_empty() {
        return Err("a tuning needs at least one string".into());
    }
    for (i, s) in entry.strings.iter().enumerate() {
        let n = i + 1;
        if s.name.trim().is_empty() {
            return Err(format!("string {n} has an empty label"));
        }
        // PresetString.midi is a u8 so the upper bound is structural; the
        // lower bound is also free. We still bail above 127 if serde gave us
        // one (impossible today, but cheap insurance) — and we treat 0 / 1
        // as "below E0" which isn't a useful tuning. Pick A0 (21) as the
        // practical floor; lower than any real string on any real instrument.
        if s.midi < 21 {
            return Err(format!(
                "string {n} pitch is below A0 (MIDI {}) — too low to be a real string",
                s.midi
            ));
        }
        if s.midi > 108 {
            return Err(format!(
                "string {n} pitch is above C8 (MIDI {}) — too high to be a real string",
                s.midi
            ));
        }
    }
    Ok(())
}

/// Run YIN pitch detection over a buffer of mono f32 samples at `sample_rate`.
/// Returns the detected frequency in Hz, or `None` if the buffer is too quiet
/// or doesn't contain a confident pitch. This is the actual pitch engine the
/// browser tuner will end up calling once we wire microphone capture.
///
/// The YIN threshold of 0.15 matches what the native `Tuner` uses — keeps the
/// browser pitch detection behaviourally identical to the CLI tuner.
#[wasm_bindgen]
pub fn detect_pitch(samples: &[f32], sample_rate: u32) -> Option<f32> {
    use twanga_dsp::PitchDetector;
    let mut yin = twanga_dsp::Yin::new(0.15);
    yin.detect(samples, sample_rate).map(|f| f.hz())
}

/// Stateful tuner — the WASM mirror of `twanga_dsp::Tuner`. Buffers incoming
/// samples across multiple `feed` calls (web audio worklets post 128-sample
/// chunks; the YIN window is ~8K samples), drains readings on demand.
///
/// Three factory constructors:
/// - `WebTuner.new_chromatic(sr)` — nearest-12-TET-note mode; one reading
///   per accepted detection, label is the note name (`A4`, `C#3`, …).
/// - `WebTuner.new_for_strings(slug, sr)` — per-string mode against the
///   built-in tuning identified by `slug` (`standard-guitar`, etc.). At most
///   one reading per string per analysis window, gated against detections
///   that aren't plausibly any string in the tuning.
/// - `WebTuner.new_for_strings_with_capo(slug, capo_spec, sr)` — as above,
///   but applies a `Capo` (parsed from `capo_spec` — `"3"` for uniform,
///   `"0,2,2,2,2,2"` for per-string) before constructing the underlying
///   tuner. Targets are the effective post-capo open pitches, matching
///   what `twanga tune --capo` does on the CLI.
#[wasm_bindgen]
pub struct WebTuner {
    inner: twanga_dsp::Tuner,
    string_labels: Vec<String>,
    /// The tuning's display name, possibly suffixed with `(capo N)` /
    /// `(partial capo)` if a capo was applied. Mirrors `Tuning.name` after
    /// `Capo::apply`.
    name: String,
}

#[derive(serde::Serialize)]
struct ReadingJs {
    label: String,
    detected_hz: f32,
    target_hz: f32,
    cents: f32,
}

#[derive(serde::Serialize)]
struct StringInfoJs {
    label: String,
    open_hz: f32,
    midi: u8,
}

#[wasm_bindgen]
impl WebTuner {
    /// Build a chromatic tuner. Use this when the user hasn't picked an
    /// instrument — readings carry the nearest-MIDI note name as the label.
    pub fn new_chromatic(sample_rate: u32) -> WebTuner {
        WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Chromatic, sample_rate),
            string_labels: Vec::new(),
            name: "Chromatic".to_string(),
        }
    }

    /// Build a per-string tuner from a built-in preset slug
    /// (`standard-guitar` / `standard-banjo` / `standard-ukulele` / `drop-d-guitar`
    /// / `tenor-banjo` / `tenor-ukulele`). Errors if the slug is unknown so
    /// the JS side surfaces a clear message rather than silently falling back.
    /// Returns `Result<_, String>` rather than `Result<_, JsError>` so the
    /// same code paths can run in `cargo test` on native targets — the
    /// `JsError::new` import panics outside wasm.
    pub fn new_for_strings(slug: &str, sample_rate: u32) -> Result<WebTuner, String> {
        let tuning = twanga_core::Tuning::from_preset(slug)
            .ok_or_else(|| format!("unknown tuning slug: {slug}"))?;
        let labels = tuning.strings.iter().map(|s| s.name.clone()).collect();
        let name = tuning.name.clone();
        Ok(WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Strings(tuning), sample_rate),
            string_labels: labels,
            name,
        })
    }

    /// Like [`Self::new_for_strings`] but applies a `Capo` first, so the
    /// targets are the post-capo open pitches. `capo_spec` accepts either a
    /// uniform integer (`"3"` — capo on fret 3) or a per-string comma list
    /// (`"0,2,2,2,2,2"` — drop-D-style partial capo). Empty / `"0"` produces
    /// a no-op capo and behaves identically to `new_for_strings`. Mirrors
    /// `twanga tune --capo` on the CLI exactly.
    pub fn new_for_strings_with_capo(
        slug: &str,
        capo_spec: &str,
        sample_rate: u32,
    ) -> Result<WebTuner, String> {
        let tuning = twanga_core::Tuning::from_preset(slug)
            .ok_or_else(|| format!("unknown tuning slug: {slug}"))?;
        let capo = twanga_core::Capo::parse(capo_spec, tuning.strings.len())?;
        let effective = capo.apply(&tuning)?;
        let labels = effective.strings.iter().map(|s| s.name.clone()).collect();
        let name = effective.name.clone();
        Ok(WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Strings(effective), sample_rate),
            string_labels: labels,
            name,
        })
    }

    /// Build a per-string tuner from a user-defined `PresetEntry`-shaped JS
    /// object (same schema the CLI persists under `$CONFIG/twanga/tunings.toml`).
    /// The frontend stores these in `localStorage` under
    /// `twanga-user-tunings-v1` so the tuner picker can include them without
    /// a server round-trip. Validation is run before construction; errors
    /// flow back to JS as a `String` exception, mirroring
    /// [`Self::new_for_strings`].
    pub fn new_for_strings_custom(
        preset_json: JsValue,
        sample_rate: u32,
    ) -> Result<WebTuner, String> {
        Self::new_for_strings_custom_with_capo(preset_json, "", sample_rate)
    }

    /// Capo'd variant of [`Self::new_for_strings_custom`]. Same `capo_spec`
    /// rules as [`Self::new_for_strings_with_capo`]: uniform integer or
    /// per-string comma list, empty / `"0"` is a no-op.
    pub fn new_for_strings_custom_with_capo(
        preset_json: JsValue,
        capo_spec: &str,
        sample_rate: u32,
    ) -> Result<WebTuner, String> {
        let entry: twanga_core::PresetEntry = serde_wasm_bindgen::from_value(preset_json)
            .map_err(|e| format!("malformed tuning shape: {e}"))?;
        Self::build_from_entry(entry, capo_spec, sample_rate)
    }

    /// Native-Rust path used by both the wasm-exported `new_for_strings_custom*`
    /// constructors and the `cargo test` suite (where building a `JsValue`
    /// outside wasm isn't possible). Same validation + capo application as the
    /// JS-facing wrappers. Lives inside the `#[wasm_bindgen]` impl block on
    /// purpose: closing the block here and reopening a plain `impl` for the
    /// rest of the methods would silently drop them from the exported JS
    /// class (cargo tests don't catch this — the macro is a no-op on
    /// native — but the deployed WASM bundle would lose `name()`,
    /// `string_labels()`, `feed()`, etc).
    fn build_from_entry(
        entry: twanga_core::PresetEntry,
        capo_spec: &str,
        sample_rate: u32,
    ) -> Result<WebTuner, String> {
        validate_entry(&entry)?;
        let tuning = entry.to_tuning();
        let capo = twanga_core::Capo::parse(capo_spec, tuning.strings.len())?;
        let effective = capo.apply(&tuning)?;
        let labels = effective.strings.iter().map(|s| s.name.clone()).collect();
        let name = effective.name.clone();
        Ok(WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Strings(effective), sample_rate),
            string_labels: labels,
            name,
        })
    }

    /// Effective display name for the active tuning. After capo application
    /// this is e.g. `"Standard Banjo (Open G) (capo 3)"`; without a capo it's
    /// the raw preset name (`"Standard Ukulele (Reentrant GCEA)"`). For
    /// chromatic-mode tuners it's `"Chromatic"`.
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// The string labels in string-number order (string 1 first). Lets the
    /// frontend render the per-string rows up-front, before any audio has
    /// been fed, so the layout doesn't shift around when the first reading
    /// arrives. Returns an empty array for chromatic-mode tuners.
    pub fn string_labels(&self) -> Vec<String> {
        self.string_labels.clone()
    }

    /// Open-string info (label, open Hz, MIDI) for each string in the
    /// per-string tuner. Returns an empty array for chromatic tuners.
    /// Useful for the UI to show "A4 — 440.00 Hz" target headers.
    pub fn strings_info(&self) -> JsValue {
        let info: Vec<StringInfoJs> = match self.inner.mode() {
            twanga_dsp::TunerMode::Strings(t) => t
                .strings
                .iter()
                .map(|s| StringInfoJs {
                    label: s.name.clone(),
                    open_hz: s.open.to_frequency().hz(),
                    midi: s.open.0,
                })
                .collect(),
            twanga_dsp::TunerMode::Chromatic => Vec::new(),
        };
        serde_wasm_bindgen::to_value(&info).unwrap()
    }

    /// Push mono f32 samples into the analysis buffer. Same call pattern as
    /// the native `Tuner::feed`. Worklet chunks (128 samples on most
    /// browsers) accumulate across calls until a YIN window fills.
    pub fn feed(&mut self, samples: &[f32]) {
        self.inner.feed(samples);
    }

    /// Drain accumulated readings. Each entry is `{ label, detected_hz,
    /// target_hz, cents }`. In `Strings` mode multiple strings can produce
    /// readings per call; the UI should treat the result as "updates for
    /// these strings" and leave others unchanged.
    pub fn take_readings(&mut self) -> JsValue {
        let readings: Vec<ReadingJs> = self
            .inner
            .take_readings()
            .map(|r| ReadingJs {
                label: r.label,
                detected_hz: r.detected.hz(),
                target_hz: r.target.hz(),
                cents: r.cents,
            })
            .collect();
        serde_wasm_bindgen::to_value(&readings).unwrap()
    }
}

#[cfg(test)]
mod tests {
    //! `#[wasm_bindgen]` macros expand to regular fn bodies when compiling for
    //! a non-wasm target, so we can `cargo test -p twanga-web` directly. These
    //! tests cover the wrapper logic — translation from Rust types to the
    //! shapes JS sees — not the underlying crates (those have their own
    //! coverage already in twanga-core / twanga-dsp).
    use super::*;

    #[test]
    fn pick_splash_returns_nonempty_and_is_stable_per_seed() {
        let a = pick_splash(0);
        let b = pick_splash(0);
        assert!(!a.is_empty(), "splash should not be empty");
        assert_eq!(a, b, "same seed must yield same splash");
    }

    #[test]
    fn pick_splash_wraps_around_modulo_list_length() {
        // Doesn't panic on huge seeds — modulo over the splash count keeps it
        // in range. Brittle assertion: just confirm it succeeds and returns
        // a known-good string. Equality with seed=0 is the proof of wrap.
        let s = pick_splash(u32::MAX);
        assert!(!s.is_empty());
    }

    #[test]
    fn builtin_tuning_slugs_matches_core_registry() {
        let from_web = builtin_tuning_slugs();
        let from_core: Vec<String> = twanga_core::Tuning::builtin_slugs()
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            from_web, from_core,
            "wasm wrapper must not reorder or drop slugs"
        );
    }

    #[test]
    fn midi_from_name_handles_canonical_notes() {
        assert_eq!(midi_from_name("A4"), Some(69));
        assert_eq!(midi_from_name("C4"), Some(60));
        assert_eq!(midi_from_name("C#3"), Some(49));
        assert_eq!(midi_from_name("E2"), Some(40));
    }

    #[test]
    fn midi_from_name_rejects_garbage() {
        assert_eq!(midi_from_name(""), None);
        assert_eq!(midi_from_name("garbage"), None);
        assert_eq!(midi_from_name("H4"), None); // no H note in 12-TET
        assert_eq!(midi_from_name("Cb4"), None); // no flats accepted
    }

    #[test]
    fn midi_to_name_is_inverse_of_from_name_across_full_range() {
        for midi in 21..=108_u8 {
            let name = midi_to_name(midi);
            assert_eq!(
                midi_from_name(&name),
                Some(midi),
                "round-trip failed for MIDI {midi} (name {name})"
            );
        }
    }

    #[test]
    fn detect_pitch_identifies_440hz_sine() {
        let sr = 48_000;
        let n = 4096;
        let buf: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let hz = detect_pitch(&buf, sr).expect("should detect a pitch");
        assert!((hz - 440.0).abs() < 1.0, "expected ~440 Hz, got {hz:.2}");
    }

    #[test]
    fn detect_pitch_rejects_silence() {
        // All-zero buffer has no pitch — YIN's threshold gate should reject
        // it and return None. This confirms the wrapper propagates the
        // detector's "no confident pitch" signal without translating to e.g.
        // 0.0 Hz, which would mislead the frontend.
        let buf = vec![0.0_f32; 4096];
        assert_eq!(detect_pitch(&buf, 48_000), None);
    }

    #[test]
    fn detect_pitch_handles_buffer_too_short_to_analyse() {
        // YIN needs at least a couple of samples; tiny buffers should return
        // None rather than panic.
        let buf = vec![0.5_f32; 4];
        let _ = detect_pitch(&buf, 48_000); // just confirm no panic
    }

    /// Helper: synthetic sine wave of `freq` Hz, length samples, normalised
    /// to ±1. Mirrors the test fixture pattern from twanga-dsp.
    fn sine(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn web_tuner_strings_labels_match_preset() {
        // Per-string-tuner labels must come back in preset order — the
        // browser UI relies on this to lay out rows top-to-bottom in
        // string-number order (string 1 first).
        let t = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(t.string_labels(), vec!["A4", "E4", "C4", "g4 (reentrant)"]);
    }

    #[test]
    fn web_tuner_rejects_unknown_slug() {
        // `WebTuner` doesn't implement Debug (no value in deriving on a
        // wasm-bindgen exported struct), so use `is_err()` rather than
        // `unwrap_err()`, which would need Debug on the success type.
        assert!(WebTuner::new_for_strings("not-a-tuning", 48_000).is_err());
    }

    #[test]
    fn web_tuner_chromatic_has_no_string_labels() {
        let t = WebTuner::new_chromatic(48_000);
        assert!(t.string_labels().is_empty());
    }

    #[test]
    fn web_tuner_strings_detects_a4_against_uke_a_string() {
        // 440 Hz fed in should produce a reading labelled "A4" (the uke's
        // first string) within a few cents of zero.
        let mut t = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        // Native Tuner needs enough samples to fill a window (8192). Feed
        // a generous run so we definitely get a reading out.
        let samples = sine(440.0, 48_000, 16_384);
        t.inner.feed(&samples);
        let readings: Vec<_> = t.inner.take_readings().collect();
        assert!(
            !readings.is_empty(),
            "expected at least one reading for 440 Hz"
        );
        let first = &readings[0];
        assert_eq!(first.label, "A4");
        assert!(
            first.cents.abs() < 5.0,
            "expected near-zero cents on the A string, got {:.2}",
            first.cents
        );
    }

    #[test]
    fn web_tuner_chromatic_identifies_440hz_as_a4() {
        let mut t = WebTuner::new_chromatic(48_000);
        let samples = sine(440.0, 48_000, 16_384);
        t.inner.feed(&samples);
        let readings: Vec<_> = t.inner.take_readings().collect();
        assert!(!readings.is_empty());
        assert_eq!(readings[0].label, "A4");
    }

    #[test]
    fn preset_display_name_returns_human_label() {
        assert_eq!(
            preset_display_name("standard-ukulele").as_deref(),
            Some("Standard Ukulele (Reentrant GCEA)"),
        );
        assert_eq!(
            preset_display_name("drop-d-guitar").as_deref(),
            Some("Drop D Guitar (DADGBE)"),
        );
        assert!(preset_display_name("not-a-tuning").is_none());
    }

    #[test]
    fn web_tuner_name_reflects_active_mode() {
        let chromatic = WebTuner::new_chromatic(48_000);
        assert_eq!(chromatic.name(), "Chromatic");

        let uke = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(uke.name(), "Standard Ukulele (Reentrant GCEA)");
    }

    #[test]
    fn web_tuner_with_capo_shifts_targets_and_relabels_strings() {
        // Uniform capo 3 on uke: every string +3 semitones. A4 → C5,
        // E4 → G4, C4 → D#4 (Eb4), reentrant g4 → A#4 (Bb4). The labels
        // come from `MidiNote::name()` of the shifted MIDI value, which
        // is `Capo::apply`'s convention when the offset is non-zero.
        let t = WebTuner::new_for_strings_with_capo("standard-ukulele", "3", 48_000).unwrap();
        let labels = t.string_labels();
        assert_eq!(labels, vec!["C5", "G4", "D#4", "A#4"]);

        // Effective name carries the capo annotation.
        assert!(
            t.name().contains("capo 3"),
            "expected '(capo 3)' in name, got {:?}",
            t.name(),
        );
    }

    #[test]
    fn web_tuner_with_zero_capo_matches_no_capo_constructor() {
        // `"0"` is the canonical no-op capo. Should produce the same labels
        // as the no-capo constructor, with the reentrant label preserved
        // (since the offset on that string is also 0).
        let with = WebTuner::new_for_strings_with_capo("standard-ukulele", "0", 48_000).unwrap();
        let without = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(with.string_labels(), without.string_labels());
        assert_eq!(with.string_labels()[3], "g4 (reentrant)");
    }

    #[test]
    fn web_tuner_with_partial_capo_preserves_unchanged_string_labels() {
        // Banjo body capo on fret 3, 5th-string drone left open. The drone
        // label `g4 (drone)` should survive because its offset is 0; the
        // body strings get relabelled to their new MIDI names.
        let t = WebTuner::new_for_strings_with_capo("standard-banjo", "3,3,3,3,0", 48_000).unwrap();
        let labels = t.string_labels();
        // Body strings: D4+3=F4, B3+3=D4, G3+3=A#3, D3+3=F3.
        assert_eq!(&labels[..4], &["F4", "D4", "A#3", "F3"]);
        // 5th-string drone label is preserved (offset 0).
        assert_eq!(labels[4], "g4 (drone)");
        assert!(t.name().contains("partial capo"));
    }

    #[test]
    fn web_tuner_with_invalid_capo_returns_err() {
        // Capo spec with wrong length should propagate the Capo::parse
        // error message up to the JS side, not panic.
        assert!(WebTuner::new_for_strings_with_capo("standard-ukulele", "1,2", 48_000).is_err());
    }

    // ------------------------------------------------------------------
    // Custom (user-defined) tuning tests
    // ------------------------------------------------------------------
    //
    // `WebTuner::new_for_strings_custom*` ultimately deserialises a JsValue
    // into `PresetEntry`, but `JsValue` can't be constructed off-wasm. The
    // wasm path is covered by manual smoke-testing in the browser; here we
    // exercise the shared `build_from_entry` helper which carries the same
    // validation + capo logic.

    use twanga_core::{PresetEntry, PresetString};

    fn sample_custom_entry() -> PresetEntry {
        // Open D guitar — deliberately not in the built-in registry.
        PresetEntry {
            slug: "open-d-test".into(),
            name: "Open D Test Tuning".into(),
            strings: vec![
                PresetString {
                    name: "D4".into(),
                    midi: 62,
                },
                PresetString {
                    name: "A3".into(),
                    midi: 57,
                },
                PresetString {
                    name: "F#3".into(),
                    midi: 54,
                },
                PresetString {
                    name: "D3".into(),
                    midi: 50,
                },
                PresetString {
                    name: "A2".into(),
                    midi: 45,
                },
                PresetString {
                    name: "D2".into(),
                    midi: 38,
                },
            ],
        }
    }

    #[test]
    fn build_from_entry_constructs_tuner_with_preset_labels() {
        let t = WebTuner::build_from_entry(sample_custom_entry(), "", 48_000).expect("build");
        assert_eq!(t.string_labels(), vec!["D4", "A3", "F#3", "D3", "A2", "D2"]);
        assert_eq!(t.name(), "Open D Test Tuning");
    }

    #[test]
    fn build_from_entry_applies_capo() {
        let t = WebTuner::build_from_entry(sample_custom_entry(), "2", 48_000).expect("build");
        // +2 semitones on every string. D4 → E4, A3 → B3, F#3 → G#3, etc.
        assert_eq!(t.string_labels(), vec!["E4", "B3", "G#3", "E3", "B2", "E2"]);
        assert!(t.name().contains("capo 2"));
    }

    #[test]
    fn validate_entry_accepts_well_formed_entry() {
        assert!(validate_entry(&sample_custom_entry()).is_ok());
    }

    #[test]
    fn validate_entry_rejects_builtin_slug() {
        let mut e = sample_custom_entry();
        e.slug = "standard-ukulele".into();
        let err = validate_entry(&e).expect_err("must refuse to shadow built-in");
        assert!(err.contains("built-in"), "unexpected error: {err}");
    }

    #[test]
    fn validate_entry_rejects_bad_slug_characters() {
        let mut e = sample_custom_entry();
        e.slug = "Open D".into(); // uppercase + space — invalid
        assert!(validate_entry(&e).is_err());

        e.slug = "-leading-hyphen".into();
        assert!(validate_entry(&e).is_err());

        e.slug = "trailing-hyphen-".into();
        assert!(validate_entry(&e).is_err());

        e.slug = String::new();
        assert!(validate_entry(&e).is_err());
    }

    #[test]
    fn validate_entry_rejects_empty_name() {
        let mut e = sample_custom_entry();
        e.name = "   ".into();
        let err = validate_entry(&e).expect_err("blank name should fail");
        assert!(err.contains("display name"), "unexpected error: {err}");
    }

    #[test]
    fn validate_entry_rejects_empty_string_list() {
        let mut e = sample_custom_entry();
        e.strings.clear();
        let err = validate_entry(&e).expect_err("zero-string tuning should fail");
        assert!(
            err.contains("at least one string"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_entry_rejects_blank_string_label() {
        let mut e = sample_custom_entry();
        e.strings[2].name = String::new();
        let err = validate_entry(&e).expect_err("blank string label should fail");
        assert!(err.contains("string 3"), "unexpected error: {err}");
    }

    // ------------------------------------------------------------------
    // Recorder-support tests (match_pitch_to_fret / serialize_recording)
    // ------------------------------------------------------------------
    //
    // The wasm-exported variants take `JsValue`, which can't be built off
    // wasm. The matching + serialisation logic itself lives in the
    // `twanga-core` / `twanga-tabs` crates (covered by their own test
    // suites), so here we just confirm the Rust-internal shape we'd produce
    // before crossing the wasm boundary stays correct.

    #[test]
    fn match_to_fret_against_capoed_tuning_matches_cli() {
        // Capo 3 on standard uke. Open A-string becomes C5 (MIDI 72).
        // Detecting C5 should match string 0 (A-string), fret 0.
        let uke = twanga_core::Tuning::standard_ukulele();
        let capo = twanga_core::Capo::uniform(uke.strings.len(), 3);
        let effective = capo.apply(&uke).unwrap();
        let c5 = twanga_core::Frequency(twanga_core::MidiNote(72).to_frequency().hz());
        let m = effective.match_to_fret(c5, 20).expect("should match");
        assert_eq!(m.string_idx, 0);
        assert_eq!(m.fret, 0);
    }

    #[test]
    fn serialize_recording_round_trips_through_alphatex_writer() {
        // Smoke test for the writer surface: 2 columns of standard-guitar,
        // no capo. The output is exercised by twanga-tabs' own tests; here
        // we just confirm the wiring writes *something* parseable and that
        // a non-capo recording doesn't accidentally embed `capo=` into the
        // subtitle.
        let guitar = twanga_core::Tuning::standard_guitar();
        let capo = twanga_core::Capo::uniform(guitar.strings.len(), 0);
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w =
                twanga_tabs::alphatex::AlphaTexWriter::new(&mut buf, &guitar, &capo, 120, 8, None)
                    .unwrap();
            w.write_column(&[Some(0), None, None, None, None, None])
                .unwrap();
            w.write_column(&[None, Some(2), None, None, None, None])
                .unwrap();
            w.finalize().unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        assert!(
            text.contains("\\tempo 120"),
            "expected tempo line in: {text}"
        );
        assert!(text.contains("\\tuning"), "expected tuning line in: {text}");
        assert!(
            !text.contains("capo="),
            "no capo should be embedded: {text}"
        );
        assert!(!text.contains("\\title"), "no title when None: {text}");
    }

    #[test]
    fn serialize_recording_emits_title_when_provided() {
        // Title flows through the WASM binding into `\title "..."` so a
        // browser-saved recording carries the user's chosen name. Same
        // writer the CLI uses, so the file is byte-identical to a CLI
        // recording with the same inputs.
        let guitar = twanga_core::Tuning::standard_guitar();
        let capo = twanga_core::Capo::uniform(guitar.strings.len(), 0);
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = twanga_tabs::alphatex::AlphaTexWriter::new(
                &mut buf,
                &guitar,
                &capo,
                120,
                8,
                Some("My Recording"),
            )
            .unwrap();
            w.finalize().unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        assert!(
            text.contains("\\title \"My Recording\""),
            "expected \\title line in: {text}"
        );
    }

    // ---- WebParsedTab parse / accessor / transpose tests ----

    #[test]
    fn parse_alphatex_round_trips_title_subtitle_tempo_tuning() {
        let input = "\\title \"Cripple Creek\"\n\
                     \\subtitle \"Standard Banjo (Open G)\"\n\
                     \\tempo 110\n\
                     \\tuning D4 B3 G3 D3 G4\n\
                     .\n\
                     :8 0.3 |\n";
        let parsed = parse_alphatex(input).expect("parse");
        assert_eq!(parsed.title().as_deref(), Some("Cripple Creek"));
        assert_eq!(
            parsed.subtitle_display().as_deref(),
            Some("Standard Banjo (Open G)")
        );
        assert_eq!(parsed.tempo(), 110);
        assert_eq!(parsed.tuning_names(), vec!["D4", "B3", "G3", "D3", "G4"]);
        assert_eq!(parsed.columns_count(), 1);
        assert_eq!(parsed.capo_spec(), "");
    }

    #[test]
    fn parse_alphatex_extracts_capo_from_subtitle() {
        let input = "\\subtitle \"Standard Banjo (Open G); capo=3,3,3,3,0\"\n\
                     \\tempo 110\n\
                     \\tuning D4 B3 G3 D3 G4\n\
                     .\n";
        let parsed = parse_alphatex(input).expect("parse");
        // Capo serialises back to the same per-string syntax the CLI uses.
        assert_eq!(parsed.capo_spec(), "3,3,3,3,0");
        // subtitle_display strips the `; capo=...` machine annotation.
        assert_eq!(
            parsed.subtitle_display().as_deref(),
            Some("Standard Banjo (Open G)")
        );
    }

    #[test]
    fn parse_alphatex_surfaces_parse_errors_as_strings() {
        // Bad `\tempo` value: the parser returns ParseError::BadTempo,
        // which we map to a `Result<_, String>` for the wasm boundary.
        // `WebParsedTab` doesn't implement Debug (no point on a
        // wasm-bindgen exported struct), so use `.err()` rather than
        // `.expect_err()`.
        let input = "\\tempo not-a-number\n\\tuning A4 E4 C4 G4\n.\n";
        let err = parse_alphatex(input).err().expect("expected parse error");
        assert!(err.contains("bad tempo"), "unexpected error: {err}");
    }

    #[test]
    fn validate_entry_rejects_out_of_range_midi() {
        // Below A0: too low for a real string.
        let mut low = sample_custom_entry();
        low.strings[0].midi = 5;
        assert!(validate_entry(&low).is_err());

        // Above C8: too high for a real string.
        let mut high = sample_custom_entry();
        high.strings[0].midi = 120;
        assert!(validate_entry(&high).is_err());
    }
}
