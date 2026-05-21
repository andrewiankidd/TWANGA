// Silence-threshold slider that overlays the mic-meter bar. Same
// factory shape as makeMicMeter / makeDevicePicker. Sliding the
// thumb adjusts the silence gate in real time on the active
// WebTuner instance — the screen wiring uses the `onChange`
// callback to push the new RMS value into the live tuner via
// `WebTuner.set_silence_rms(rms)`.
//
// The slider's 0..100 value maps to dB on the same scale the meter
// uses (NOISE_FLOOR_DB = -60 → 0%, 0 dB → 100%), which gives the
// user the same axis to think in: the bar fill shows the live
// signal in dB; the slider thumb shows the silence threshold in
// dB on the same axis. When the fill crosses the thumb, detection
// is happening.
//
// Required markup (matches `.mic-meter-threshold` CSS in app.html):
//
//   <input type="range" id="..." class="mic-meter-threshold"
//          min="0" max="100" step="1">

const NOISE_FLOOR_DB = -60;
const DEFAULT_RMS = 0.005; // Matches twanga-dsp's Tuner::DEFAULT_SILENCE_RMS.

export function makeSilenceThreshold({ sliderId, storageKey, onChange }) {
    const slider = document.getElementById(sliderId);
    if (!slider) {
        throw new Error(`makeSilenceThreshold: missing element ${sliderId}`);
    }

    let rms = loadRms(storageKey);
    slider.value = String(Math.round(rmsToPct(rms)));

    // `input` (not `change`) so dragging fires continuously and the
    // gate updates as the user adjusts — same UX as a hardware
    // noise-gate threshold knob.
    slider.addEventListener('input', () => {
        const pct = Number(slider.value);
        rms = pctToRms(pct);
        saveRms(storageKey, rms);
        if (typeof onChange === 'function') {
            onChange(rms);
        }
    });

    return {
        getRms() { return rms; },
        // Programmatic update (e.g. CLI sync via Tauri command in
        // the future). Updates slider position + persisted value
        // + the on-tuner state, mirroring user drag.
        setRms(newRms) {
            rms = Math.max(0, Math.min(1, newRms));
            slider.value = String(Math.round(rmsToPct(rms)));
            saveRms(storageKey, rms);
            if (typeof onChange === 'function') {
                onChange(rms);
            }
        },
    };
}

function pctToRms(pct) {
    if (pct <= 0) return 0;
    const dB = (pct / 100) * (-NOISE_FLOOR_DB) + NOISE_FLOOR_DB;
    return Math.pow(10, dB / 20);
}

function rmsToPct(rms) {
    if (rms <= 0) return 0;
    const dB = 20 * Math.log10(rms);
    const pct = ((dB - NOISE_FLOOR_DB) / -NOISE_FLOOR_DB) * 100;
    return Math.max(0, Math.min(100, pct));
}

function loadRms(storageKey) {
    try {
        const raw = localStorage.getItem(storageKey);
        if (raw === null) return DEFAULT_RMS;
        const n = parseFloat(raw);
        return isFinite(n) && n >= 0 && n <= 1 ? n : DEFAULT_RMS;
    } catch (e) {
        return DEFAULT_RMS;
    }
}

function saveRms(storageKey, rms) {
    try {
        localStorage.setItem(storageKey, String(rms));
    } catch (e) {
        console.warn('saveRms failed', e);
    }
}
