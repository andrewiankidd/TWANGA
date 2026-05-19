// AudioWorklet processor for TWANGA's browser tuner.
//
// Runs on Web Audio's separate render thread (NOT the main JS thread) and
// receives mic samples in 128-sample chunks — the platform's "render quantum".
// We buffer up to FRAME samples (~85 ms at 48 kHz; same order of magnitude
// the native CLI tuner uses for its YIN window) before posting the full
// buffer back to the main thread, where the WASM-compiled YIN detector
// chews on it.
//
// Why we don't just call WASM from here: AudioWorkletProcessors can't import
// from the main thread, and instantiating a separate WASM module per worklet
// would mean a second copy of `twanga-dsp` running in audio-thread context.
// Posting samples back over `port.postMessage` (copy-by-default) is fine for
// our throughput (~12 detections/sec at the default frame size).

const FRAME = 4096;

class PitchProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        this.buffer = new Float32Array(FRAME);
        this.index = 0;
    }

    process(inputs) {
        // Web Audio gives us a [input][channel][sample] cube. We take the
        // first input's first channel (mono) — getUserMedia({audio: true})
        // on most browsers yields a single-channel stream anyway.
        const channel = inputs[0]?.[0];
        if (!channel) return true; // keep the worklet alive even if upstream is silent

        for (let i = 0; i < channel.length; i++) {
            this.buffer[this.index++] = channel[i];
            if (this.index >= FRAME) {
                // postMessage copies by default; main thread gets a fresh
                // Float32Array it can hand to the WASM bindings without
                // worrying about us mutating it underneath.
                this.port.postMessage(this.buffer.slice(0));
                this.index = 0;
            }
        }

        return true;
    }
}

registerProcessor('pitch-processor', PitchProcessor);
