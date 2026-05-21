// Shared mic-level meter used by the Tuner, Recorder, and Playback
// (wait mode) screens. Diagnostic + UX feedback so the user can tell
// "audio is flowing, pitch detection is just disagreeing" apart from
// "no audio at all" (permission denied, OS-level mute, suspended
// AudioContext) — and now also "what does the detector think I'm
// playing?".
//
// Factory pattern matching `makeTuningController` — each screen owns
// its own instance, passes in the DOM ids of its meter markup, and
// pipes its mic-callback samples through `feed(samples)` to drive the
// RMS reading. When the screen's reading loop sees a detected pitch
// (i.e. the WebTuner's silence + YIN gates both opened), it calls
// `setDetectedNote(label)` to populate the note pill. The pill fades
// automatically after a short timeout if no further readings arrive
// — useful for telling "I'm playing but the detector can't lock"
// apart from "I'm playing the wrong note".
//
// The RAF loop paints the latest values at display rate and surfaces
// a "no signal" hint after a couple of seconds of no audio chunks.
//
// Required markup (matches `.mic-meter` CSS in app.html):
//
//   <div class="mic-meter" id="..."> hidden
//     <span class="mic-meter-label">mic</span>
//     <div class="mic-meter-bar-wrap">
//       <div class="mic-meter-bar">
//         <div class="mic-meter-fill" id="..."></div>
//       </div>
//       <input type="range" class="mic-meter-threshold" id="...">
//     </div>
//     <span class="mic-meter-db" id="...">—</span>
//     <span class="mic-meter-note" id="..."></span>   <!-- optional -->
//   </div>

const NO_SIGNAL_TIMEOUT_MS = 2000;
const NOISE_FLOOR_DB = -60;
// How long after the last setDetectedNote() call to keep the pill
// visible. 800 ms covers brief gaps between plucks on short-sustain
// instruments (banjo / uke) while still clearing within ~1s of the
// user stopping. The silence gate already drops readings during true
// silence; this is only a UI fade.
const NOTE_FADE_MS = 800;

export function makeMicMeter({ meterId, fillId, dbId, noteId, consumerName, getActiveConsumer }) {
    const meter = document.getElementById(meterId);
    const fill = document.getElementById(fillId);
    const dbEl = document.getElementById(dbId);
    // `noteId` is optional — callers that don't pass it keep the
    // pre-existing meter behaviour (RMS bar + dB readout only).
    const noteEl = noteId ? document.getElementById(noteId) : null;
    if (!meter || !fill || !dbEl) {
        throw new Error(`makeMicMeter: missing element(s) — ${meterId}/${fillId}/${dbId}`);
    }

    let micLevel = 0;
    let micLastSampleAt = 0;
    let detectedLabel = null;
    let detectedAt = 0;
    let rafHandle = null;
    let startedAt = 0;

    function feed(samples) {
        let sum = 0;
        for (let i = 0; i < samples.length; i++) {
            sum += samples[i] * samples[i];
        }
        micLevel = Math.sqrt(sum / samples.length);
        micLastSampleAt = performance.now();
    }

    // Called by the screen's reading loop whenever the WebTuner
    // produces a reading. The label is whatever the WebTuner emitted
    // (a chromatic note name like "A4" / "C#3" in chromatic mode, or
    // a string name like "E4" / "g4 (reentrant)" in strings mode).
    // The silence gate already prevented this from firing on silence,
    // so no extra threshold check is needed here.
    function setDetectedNote(label) {
        detectedLabel = label;
        detectedAt = performance.now();
    }

    function tick() {
        if (typeof getActiveConsumer === 'function' && getActiveConsumer() !== consumerName) {
            meter.hidden = true;
            rafHandle = null;
            return;
        }
        const now = performance.now();
        const sinceSample = micLastSampleAt > 0 ? now - micLastSampleAt : now - startedAt;
        const dB = micLevel > 1e-6 ? 20 * Math.log10(micLevel) : -Infinity;
        const pct = !isFinite(dB)
            ? 0
            : Math.max(0, Math.min(100, (dB - NOISE_FLOOR_DB) * (100 / -NOISE_FLOOR_DB)));
        fill.style.width = `${pct}%`;
        if (micLastSampleAt === 0 && sinceSample > NO_SIGNAL_TIMEOUT_MS) {
            dbEl.textContent = 'no signal';
            meter.classList.add('is-silent');
        } else if (!isFinite(dB)) {
            dbEl.textContent = '—';
            meter.classList.remove('is-silent');
        } else {
            dbEl.textContent = `${dB.toFixed(1)} dB`;
            meter.classList.remove('is-silent');
        }
        if (noteEl) {
            const noteAge = now - detectedAt;
            noteEl.textContent =
                detectedLabel && detectedAt > 0 && noteAge < NOTE_FADE_MS
                    ? detectedLabel
                    : '';
        }
        rafHandle = requestAnimationFrame(tick);
    }

    function start() {
        meter.hidden = false;
        micLevel = 0;
        micLastSampleAt = 0;
        detectedLabel = null;
        detectedAt = 0;
        startedAt = performance.now();
        if (rafHandle) cancelAnimationFrame(rafHandle);
        rafHandle = requestAnimationFrame(tick);
    }

    function stop() {
        if (rafHandle) {
            cancelAnimationFrame(rafHandle);
            rafHandle = null;
        }
        meter.hidden = true;
        meter.classList.remove('is-silent');
        if (noteEl) noteEl.textContent = '';
    }

    return { feed, setDetectedNote, start, stop };
}
