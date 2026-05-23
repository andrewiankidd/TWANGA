// Pure helpers for the Playback screen's wait-mode match logic.
//
// Mirrors the Rust-side contract in `twanga-cli`'s
// `wait_reading_advances` — extracted out of the playback loop's
// mic callback so the contract is unit-testable without spinning up
// the WASM bridge, AudioWorklet, or DOM. Both sides (CLI + web)
// implement the same rule: a wait-mode reading advances the cursor
// IFF it came from a YIN window freshly anchored at a note attack
// AND its detected pitch matches one of the expected hits for the
// column the playhead is waiting on.

/// Decide whether a single tuner reading should advance the
/// wait-mode cursor.
///
/// `reading` is the shape `WebTuner.take_readings()` returns:
/// `{ label, detected_hz, target_hz, cents, from_onset_window }`.
/// `matchFn` is a closure that takes a frequency (Hz) and returns
/// whether it matches the column the playhead is currently waiting
/// on. The Playback screen builds `matchFn` from `playbackWaitMatches`
/// + the active `waitingForCol`.
///
/// Pure function — same inputs always give the same answer. No
/// reference to DOM, WASM, or `playbackState`.
export function waitReadingAdvances(reading, matchFn) {
    if (!reading || typeof matchFn !== 'function') return false;
    if (!reading.from_onset_window) return false;
    return Boolean(matchFn(reading.detected_hz));
}

/// Walk an array of tuner readings in order and return the first one
/// that should advance the wait-mode cursor, or `null` if none.
/// Mirrors the `for r in tuner.take_readings()` loop in the CLI's
/// `wait_for_expected_note`.
///
/// Returns the reading itself (not a boolean) so the caller can read
/// e.g. `detected_hz` for diagnostic surfaces (the "matched on X Hz"
/// hint that future work might surface in the UI).
export function findFirstAdvancingReading(readings, matchFn) {
    if (!Array.isArray(readings)) return null;
    for (const reading of readings) {
        if (waitReadingAdvances(reading, matchFn)) return reading;
    }
    return null;
}
