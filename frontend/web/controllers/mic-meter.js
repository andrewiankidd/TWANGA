// Shared mic-level meter used by the Recorder and Playback (wait mode)
// screens. Diagnostic + UX feedback so the user can tell "audio is
// flowing, pitch detection is just disagreeing" apart from "no audio
// at all" (permission denied, OS-level mute, suspended AudioContext).
//
// Factory pattern matching `makeTuningController` — each screen owns
// its own instance, passes in the DOM ids of its meter markup, and
// pipes its mic-callback samples through `feed(samples)` to drive the
// RMS reading. The RAF loop paints the latest value at display rate
// and surfaces a "no signal" hint after a couple of seconds of no
// audio chunks arriving.
//
// Required markup (matches `.mic-meter` CSS in app.html):
//
//   <div class="mic-meter" id="..."> hidden
//     <span class="mic-meter-label">mic</span>
//     <div class="mic-meter-bar">
//       <div class="mic-meter-fill" id="..."></div>
//     </div>
//     <span class="mic-meter-db" id="...">—</span>
//   </div>

const NO_SIGNAL_TIMEOUT_MS = 2000;
const NOISE_FLOOR_DB = -60;

export function makeMicMeter({ meterId, fillId, dbId, consumerName, getActiveConsumer }) {
    const meter = document.getElementById(meterId);
    const fill = document.getElementById(fillId);
    const dbEl = document.getElementById(dbId);
    if (!meter || !fill || !dbEl) {
        throw new Error(`makeMicMeter: missing element(s) — ${meterId}/${fillId}/${dbId}`);
    }

    let micLevel = 0;
    let micLastSampleAt = 0;
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
        rafHandle = requestAnimationFrame(tick);
    }

    function start() {
        meter.hidden = false;
        micLevel = 0;
        micLastSampleAt = 0;
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
    }

    return { feed, start, stop };
}
